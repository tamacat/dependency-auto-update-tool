package org.tamacat.depupdate;

import java.io.IOException;

/**
 * The git operations Detector and Applier need. Extracted as an interface so
 * both can be unit-tested without a real git process.
 */
public interface GitClient {

	/** The currently checked-out branch name (e.g. via {@code git rev-parse --abbrev-ref HEAD}). */
	String currentBranch() throws IOException, InterruptedException;

	/** The raw {@code git status --porcelain} output; empty/blank means a clean tree. */
	String statusPorcelain() throws IOException, InterruptedException;

	/**
	 * Stages exactly {@code relativeFilePath} and commits it with
	 * {@code message}. Never throws for a git-level failure (a non-zero exit
	 * from {@code git add}/{@code git commit}) -- that is reported as
	 * {@code CommitOutcome.success() == false} with a diagnostic. May throw
	 * for a process-level failure (e.g. {@code git} itself is not on PATH).
	 */
	CommitOutcome commitFile(String relativeFilePath, String message) throws IOException, InterruptedException;

	/**
	 * Pushes the current branch. Never throws for a git-level failure (a
	 * non-zero exit from {@code git push}) -- that is reported as
	 * {@code PushOutcome.success() == false} with a diagnostic.
	 */
	PushOutcome push() throws IOException, InterruptedException;
}

record CommitOutcome(boolean success, String commitId, String diagnostic) {
}

record PushOutcome(boolean success, String diagnostic) {
}
