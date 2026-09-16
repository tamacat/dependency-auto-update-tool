package org.tamacat.depupdate;

import java.util.Map;
import java.util.OptionalInt;

/**
 * A fixed, two-entry mapping, verified against the live repository
 * (business-rules.md BR-3): the branch -&gt; expected JDK major version
 * table. Any other checked-out branch is not a recognized target for this
 * tool.
 */
public final class BranchJdkBaseline {

	private static final Map<String, Integer> EXPECTED_MAJOR_VERSION_BY_BRANCH = Map.of(
			"master", 8,
			"v2.0-tc11", 25);

	private BranchJdkBaseline() {
	}

	public static OptionalInt expectedMajorVersion(String branch) {
		Integer version = EXPECTED_MAJOR_VERSION_BY_BRANCH.get(branch);
		return version == null ? OptionalInt.empty() : OptionalInt.of(version);
	}

	public static boolean isRecognizedBranch(String branch) {
		return EXPECTED_MAJOR_VERSION_BY_BRANCH.containsKey(branch);
	}
}
