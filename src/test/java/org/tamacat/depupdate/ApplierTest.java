package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertThrows;

import org.junit.jupiter.api.Test;

/** All four {@link ApplyResult} outcomes (ADR-11), with git operations faked. */
class ApplierTest {

	private static ValidationResult greenResultFor(String currentVersion, String candidateVersion) {
		DependencyCoordinate coord = new DependencyCoordinate("org.example", "widget");
		VersionTarget target = new DirectDependencyTarget(coord, currentVersion);
		UpdateCandidate candidate = new UpdateCandidate(target, currentVersion, candidateVersion,
				VersionClassification.STABLE);
		return new ValidationResult(candidate, ValidationOutcome.GREEN, "");
	}

	@Test
	void apply_localCommitManualApprovalMode_commitSucceeds_returnsCommittedNoPushAttempted() {
		FakeGitClient git = new FakeGitClient();
		git.enqueueCommitOutcome(new CommitOutcome(true, "abc123", null));
		DefaultApplier applier = new DefaultApplier(git);

		ApplyResult result = applier.apply(greenResultFor("1.0.0", "1.0.1"), CommitPushMode.LOCAL_COMMIT_MANUAL_APPROVAL);

		CommittedNoPushAttempted committed = assertInstanceOf(CommittedNoPushAttempted.class, result);
		assertEquals("abc123", committed.commitId());
		assertEquals(0, git.pushCallCount, "manual-approval mode must never attempt a push");
	}

	@Test
	void apply_commitItselfFails_returnsCommitFailed_andNeverAttemptsPush() {
		FakeGitClient git = new FakeGitClient();
		git.enqueueCommitOutcome(new CommitOutcome(false, null, "no git identity configured"));
		DefaultApplier applier = new DefaultApplier(git);

		ApplyResult result = applier.apply(greenResultFor("1.0.0", "1.0.1"), CommitPushMode.FULL_LOOP_AUTO_PUSH);

		CommitFailed failed = assertInstanceOf(CommitFailed.class, result);
		assertEquals("no git identity configured", failed.diagnostic());
		assertEquals(0, git.pushCallCount, "a failed commit must never lead to a push attempt (BR-6)");
	}

	@Test
	void apply_fullLoopAutoPushMode_commitAndPushSucceed_returnsCommittedAndPushed() {
		FakeGitClient git = new FakeGitClient();
		git.enqueueCommitOutcome(new CommitOutcome(true, "def456", null));
		git.enqueuePushOutcome(new PushOutcome(true, null));
		DefaultApplier applier = new DefaultApplier(git);

		ApplyResult result = applier.apply(greenResultFor("1.0.0", "1.0.1"), CommitPushMode.FULL_LOOP_AUTO_PUSH);

		CommittedAndPushed pushed = assertInstanceOf(CommittedAndPushed.class, result);
		assertEquals("def456", pushed.commitId());
	}

	@Test
	void apply_fullLoopAutoPushMode_commitSucceedsButPushFails_returnsCommittedPushFailed() {
		FakeGitClient git = new FakeGitClient();
		git.enqueueCommitOutcome(new CommitOutcome(true, "ghi789", null));
		git.enqueuePushOutcome(new PushOutcome(false, "remote rejected (auth expired)"));
		DefaultApplier applier = new DefaultApplier(git);

		ApplyResult result = applier.apply(greenResultFor("1.0.0", "1.0.1"), CommitPushMode.FULL_LOOP_AUTO_PUSH);

		CommittedPushFailed failed = assertInstanceOf(CommittedPushFailed.class, result);
		assertEquals("ghi789", failed.commitId(), "the commit is durable even though the push failed");
		assertEquals("remote rejected (auth expired)", failed.diagnostic());
	}

	@Test
	void apply_nonGreenValidationResult_throwsIllegalArgumentException() {
		DependencyCoordinate coord = new DependencyCoordinate("org.example", "widget");
		VersionTarget target = new DirectDependencyTarget(coord, "1.0.0");
		UpdateCandidate candidate = new UpdateCandidate(target, "1.0.0", "1.0.1", VersionClassification.STABLE);
		ValidationResult notGreen = new ValidationResult(candidate, ValidationOutcome.INCOMPATIBLE, "test failed");

		DefaultApplier applier = new DefaultApplier(new FakeGitClient());

		assertThrows(IllegalArgumentException.class,
				() -> applier.apply(notGreen, CommitPushMode.LOCAL_COMMIT_MANUAL_APPROVAL));
	}
}
