package org.tamacat.depupdate;

import java.util.ArrayDeque;
import java.util.Deque;

/**
 * Hand-rolled {@link GitClient} test double -- no mocking framework
 * dependency is introduced for this small, four-method interface.
 */
final class FakeGitClient implements GitClient {

	String branch = "master";
	String statusPorcelainOutput = "";
	private final Deque<CommitOutcome> commitOutcomes = new ArrayDeque<>();
	private final Deque<PushOutcome> pushOutcomes = new ArrayDeque<>();
	int commitCallCount = 0;
	int pushCallCount = 0;

	void enqueueCommitOutcome(CommitOutcome outcome) {
		commitOutcomes.addLast(outcome);
	}

	void enqueuePushOutcome(PushOutcome outcome) {
		pushOutcomes.addLast(outcome);
	}

	@Override
	public String currentBranch() {
		return branch;
	}

	@Override
	public String statusPorcelain() {
		return statusPorcelainOutput;
	}

	@Override
	public CommitOutcome commitFile(String relativeFilePath, String message) {
		commitCallCount++;
		if (commitOutcomes.isEmpty()) {
			return new CommitOutcome(true, "deadbeef", null);
		}
		return commitOutcomes.removeFirst();
	}

	@Override
	public PushOutcome push() {
		pushCallCount++;
		if (pushOutcomes.isEmpty()) {
			return new PushOutcome(true, null);
		}
		return pushOutcomes.removeFirst();
	}
}
