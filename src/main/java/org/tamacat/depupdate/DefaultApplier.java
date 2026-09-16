package org.tamacat.depupdate;

import java.io.IOException;

/**
 * Real {@link Applier}: business-rules.md BR-6/BR-7 -- commit; push only in
 * {@code FULL_LOOP_AUTO_PUSH} mode and only after a successful commit; never
 * throws; no retry on failure.
 */
public final class DefaultApplier implements Applier {

	private static final String POM_RELATIVE_PATH = "pom.xml";

	private final GitClient gitClient;

	public DefaultApplier(GitClient gitClient) {
		this.gitClient = gitClient;
	}

	@Override
	public ApplyResult apply(ValidationResult greenResult, CommitPushMode mode) {
		if (greenResult == null || greenResult.outcome() != ValidationOutcome.GREEN) {
			throw new IllegalArgumentException("Applier.apply() requires a GREEN ValidationResult");
		}
		UpdateCandidate candidate = greenResult.candidate();
		String message = CommitMessages.forCandidate(candidate);

		CommitOutcome commitOutcome;
		try {
			commitOutcome = gitClient.commitFile(POM_RELATIVE_PATH, message);
		} catch (IOException | InterruptedException e) {
			if (e instanceof InterruptedException) {
				// §12a review iteration 2: restore the interrupt status rather than
				// swallowing it (the same fix already applied to DefaultDetector/
				// DefaultValidator for Finding 6 -- this class was missed then).
				Thread.currentThread().interrupt();
			}
			return new CommitFailed("git commit threw an exception: " + e.getMessage());
		}
		if (!commitOutcome.success()) {
			return new CommitFailed(commitOutcome.diagnostic());
		}
		String commitId = commitOutcome.commitId();

		if (mode == CommitPushMode.LOCAL_COMMIT_MANUAL_APPROVAL) {
			return new CommittedNoPushAttempted(commitId);
		}

		// FULL_LOOP_AUTO_PUSH -- only reached after a successful commit (BR-6).
		try {
			PushOutcome pushOutcome = gitClient.push();
			if (pushOutcome.success()) {
				return new CommittedAndPushed(commitId);
			}
			return new CommittedPushFailed(commitId, pushOutcome.diagnostic());
		} catch (IOException | InterruptedException e) {
			if (e instanceof InterruptedException) {
				Thread.currentThread().interrupt();
			}
			return new CommittedPushFailed(commitId, "git push threw an exception: " + e.getMessage());
		}
	}
}
