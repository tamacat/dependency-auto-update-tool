package org.tamacat.depupdate;

import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Instant;
import java.time.ZoneOffset;
import java.time.format.DateTimeFormatter;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Collectors;

/**
 * Real {@link Reporter}: mirrors every recorded outcome to the console and
 * to a structured, line-oriented report file (reliability-requirements.md
 * Q5: B -- "machine-parseable... not necessarily JSON").
 *
 * <p>Report file format decision (Code Generation's own choice, per
 * reliability-requirements.md's deferral): one {@code KEY=value} record per
 * line, tagged by record type ({@code DETECTION}, {@code RESULT},
 * {@code ENVIRONMENT_FAILURE}), written under {@code ToolConfiguration.reportDirectory()}
 * as {@code dependency-auto-update-report-<yyyyMMdd-HHmmss>.log}.
 */
public final class DefaultReporter implements Reporter {

	private static final DateTimeFormatter TIMESTAMP = DateTimeFormatter.ISO_INSTANT;
	private static final DateTimeFormatter FILENAME_TIMESTAMP =
			DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss").withZone(ZoneOffset.UTC);

	private final PrintStream console;
	private final Path reportDirectory;
	private final List<String> lines = new ArrayList<>();
	private final Instant startedAt = Instant.now();

	public DefaultReporter(PrintStream console, Path reportDirectory) {
		this.console = console;
		this.reportDirectory = reportDirectory;
		emit("# dependency-auto-update-tool report -- started " + TIMESTAMP.format(startedAt));
	}

	@Override
	public void recordDetection(DetectionResult result) {
		if (result instanceof CandidateFound cf) {
			UpdateCandidate c = cf.candidate();
			emit(String.format(
					"DETECTION target=\"%s\" outcome=CANDIDATE_FOUND current=%s candidate=%s classification=%s",
					describe(c.target()), c.currentVersion(), c.candidateVersion(), c.classification()));
		} else if (result instanceof NoUpdateAvailable nu) {
			emit(String.format("DETECTION target=\"%s\" outcome=NO_UPDATE_AVAILABLE", describe(nu.target())));
		} else if (result instanceof QueryFailed qf) {
			emit(String.format("DETECTION target=\"%s\" outcome=QUERY_FAILED diagnostic=\"%s\"",
					describe(qf.target()), sanitize(qf.diagnostic())));
		}
	}

	@Override
	public void record(UpdateCandidate candidate, ValidationResult validationResult, ApplyResult applyResult) {
		StringBuilder sb = new StringBuilder();
		sb.append("RESULT target=\"").append(describe(candidate.target())).append('"')
				.append(" current=").append(candidate.currentVersion())
				.append(" candidate=").append(candidate.candidateVersion())
				.append(" classification=").append(candidate.classification())
				.append(" validation=").append(validationResult.outcome());
		if (validationResult.outcome() != ValidationOutcome.GREEN) {
			sb.append(" validationDiagnostic=\"").append(sanitize(validationResult.diagnostic())).append('"');
		}
		if (applyResult != null) {
			appendApplyResult(sb, applyResult);
		}
		emit(sb.toString());
	}

	@Override
	public void recordEnvironmentFailure(EnvironmentCheckException failure) {
		String type = failure instanceof BranchJdkMismatchException ? "BRANCH_JDK_MISMATCH" : "DIRTY_WORKING_TREE";
		emit(String.format("ENVIRONMENT_FAILURE type=%s message=\"%s\"", type, sanitize(failure.getMessage())));
	}

	@Override
	public void finish() {
		emit("# run finished " + TIMESTAMP.format(Instant.now()));
		try {
			Files.createDirectories(reportDirectory);
			String filename = "dependency-auto-update-report-" + FILENAME_TIMESTAMP.format(startedAt) + ".log";
			Path reportFile = reportDirectory.resolve(filename);
			Files.write(reportFile, lines, StandardCharsets.UTF_8);
			console.println("Report written to " + reportFile.toAbsolutePath());
		} catch (IOException e) {
			console.println("WARNING: failed to write report file: " + e.getMessage());
		}
	}

	private void emit(String line) {
		console.println(line);
		lines.add(line);
	}

	private static void appendApplyResult(StringBuilder sb, ApplyResult applyResult) {
		if (applyResult instanceof CommitFailed cf) {
			sb.append(" apply=COMMIT_FAILED applyDiagnostic=\"").append(sanitize(cf.diagnostic())).append('"');
		} else if (applyResult instanceof CommittedNoPushAttempted c) {
			sb.append(" apply=COMMITTED_NO_PUSH_ATTEMPTED commitId=").append(c.commitId());
		} else if (applyResult instanceof CommittedAndPushed c) {
			sb.append(" apply=COMMITTED_AND_PUSHED commitId=").append(c.commitId());
		} else if (applyResult instanceof CommittedPushFailed c) {
			sb.append(" apply=COMMITTED_PUSH_FAILED commitId=").append(c.commitId())
					.append(" applyDiagnostic=\"").append(sanitize(c.diagnostic())).append('"');
		}
	}

	private static String describe(VersionTarget target) {
		if (target instanceof DirectDependencyTarget ddt) {
			return ddt.coordinate().toString();
		}
		if (target instanceof PropertyIndirectedTarget pit) {
			String members = pit.members().stream().map(DependencyCoordinate::toString)
					.collect(Collectors.joining(","));
			return "${" + pit.propertyName() + "}[" + members + "]";
		}
		return String.valueOf(target);
	}

	private static String sanitize(String s) {
		if (s == null) {
			return "";
		}
		return s.replace("\"", "'").replace("\r", " ").replace("\n", " \\n ");
	}
}
