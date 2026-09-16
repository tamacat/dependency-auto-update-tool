package org.tamacat.depupdate;

import java.nio.file.Path;
import java.util.ArrayDeque;
import java.util.Deque;

/** Hand-rolled {@link BuildRunner} test double: returns queued, canned results without spawning a process. */
final class FakeBuildRunner implements BuildRunner {

	private final Deque<BuildResult> queued = new ArrayDeque<>();
	private BuildResult defaultResult = new BuildResult(0, "BUILD SUCCESS");
	int runCount = 0;

	FakeBuildRunner enqueue(BuildResult result) {
		queued.addLast(result);
		return this;
	}

	FakeBuildRunner defaultingTo(BuildResult result) {
		this.defaultResult = result;
		return this;
	}

	@Override
	public BuildResult run(Path workingDirectory, Path jdkHome) {
		runCount++;
		return queued.isEmpty() ? defaultResult : queued.removeFirst();
	}
}
