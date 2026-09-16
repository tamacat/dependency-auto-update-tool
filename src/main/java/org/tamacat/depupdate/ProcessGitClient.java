package org.tamacat.depupdate;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.concurrent.TimeUnit;

/**
 * Real {@link GitClient}: shells out to the {@code git} binary on PATH in
 * {@code repoRoot}.
 */
public final class ProcessGitClient implements GitClient {

	// Code-generation review Finding 3 (2026-09-05): an unbounded git
	// subprocess -- most plausibly `push` against a remote with no cached
	// credential helper -- could hang this tool indefinitely with no
	// diagnostic. 60s matches the upper end of the repository client's own
	// configurable timeout range (NFR Requirements).
	private static final long TIMEOUT_SECONDS = 60;

	private final Path repoRoot;

	public ProcessGitClient(Path repoRoot) {
		this.repoRoot = repoRoot;
	}

	@Override
	public String currentBranch() throws IOException, InterruptedException {
		ProcessResult r = run("git", "rev-parse", "--abbrev-ref", "HEAD");
		if (r.exitCode() != 0) {
			throw new IOException("git rev-parse --abbrev-ref HEAD failed (exit " + r.exitCode() + "): " + r.output());
		}
		return r.output().trim();
	}

	@Override
	public String statusPorcelain() throws IOException, InterruptedException {
		ProcessResult r = run("git", "status", "--porcelain");
		if (r.exitCode() != 0) {
			throw new IOException("git status --porcelain failed (exit " + r.exitCode() + "): " + r.output());
		}
		return r.output();
	}

	@Override
	public CommitOutcome commitFile(String relativeFilePath, String message) throws IOException, InterruptedException {
		ProcessResult add = run("git", "add", "--", relativeFilePath);
		if (add.exitCode() != 0) {
			return new CommitOutcome(false, null, "git add failed (exit " + add.exitCode() + "): " + add.output());
		}
		ProcessResult commit = run("git", "commit", "-m", message);
		if (commit.exitCode() != 0) {
			return new CommitOutcome(false, null,
					"git commit failed (exit " + commit.exitCode() + "): " + commit.output());
		}
		ProcessResult rev = run("git", "rev-parse", "HEAD");
		String commitId = rev.exitCode() == 0 ? rev.output().trim() : "<unknown - rev-parse failed after commit>";
		return new CommitOutcome(true, commitId, null);
	}

	@Override
	public PushOutcome push() throws IOException, InterruptedException {
		ProcessResult r = run("git", "push");
		if (r.exitCode() != 0) {
			return new PushOutcome(false, "git push failed (exit " + r.exitCode() + "): " + r.output());
		}
		return new PushOutcome(true, null);
	}

	private ProcessResult run(String... command) throws IOException, InterruptedException {
		ProcessBuilder pb = new ProcessBuilder(command);
		pb.directory(repoRoot.toFile());
		pb.redirectErrorStream(true);
		Process process = pb.start();
		// A credential-prompting `git push`/`commit` would otherwise block
		// forever reading a stdin this process never supplies -- close our end
		// immediately so any such read sees EOF and fails fast instead of
		// hanging (Finding 3).
		process.getOutputStream().close();

		ByteArrayOutputStream captured = new ByteArrayOutputStream();
		Thread drain = new Thread(() -> {
			try {
				process.getInputStream().transferTo(captured);
			} catch (IOException e) {
				// Best-effort: a forcibly-destroyed process closes the stream mid-read.
			}
		}, "git-output-drain");
		drain.setDaemon(true);
		drain.start();

		boolean finished = process.waitFor(TIMEOUT_SECONDS, TimeUnit.SECONDS);
		if (!finished) {
			process.destroyForcibly();
			drain.join(TimeUnit.SECONDS.toMillis(5));
			throw new IOException("git " + String.join(" ", command) + " timed out after " + TIMEOUT_SECONDS
					+ "s and was forcibly terminated -- check for an interactive credential prompt or a stalled network operation");
		}
		drain.join(TimeUnit.SECONDS.toMillis(5));
		return new ProcessResult(process.exitValue(), captured.toString(StandardCharsets.UTF_8));
	}

	private record ProcessResult(int exitCode, String output) {
	}
}
