package org.tamacat.depupdate;

/**
 * A closed set of two variants (Application Design ADR-7), thrown by
 * {@code Detector.validateEnvironment()} before any dependency is enumerated.
 * Callers must route this to {@code Reporter.recordEnvironmentFailure()}
 * before stopping the run.
 */
public abstract sealed class EnvironmentCheckException extends Exception
		permits BranchJdkMismatchException, DirtyWorkingTreeException {

	protected EnvironmentCheckException(String message) {
		super(message);
	}
}

/**
 * The configured JDK home for the detected branch is missing, or reports a
 * major version other than that branch's expected baseline (business-rules.md
 * BR-3). {@code configuredJdkHomeVersionOrMissing} is the major version
 * reported by the branch's <em>configured JDK home</em>, or a sentinel noting
 * the home is unset/missing -- <strong>not</strong> the tool's own running
 * JVM (corrected at NFR Requirements, 2026-09-04T23:41:28Z). Covers both "no
 * JDK home configured for this branch" and "configured JDK home reports the
 * wrong version" as the same variant, since both are the same actionable
 * failure for the maintainer: fix the machine-local JDK-home configuration.
 */
final class BranchJdkMismatchException extends EnvironmentCheckException {

	private final String detectedBranch;
	private final String configuredJdkHomeVersionOrMissing;
	private final String expectedJdkVersion;

	BranchJdkMismatchException(String detectedBranch, String configuredJdkHomeVersionOrMissing,
			String expectedJdkVersion) {
		super("Branch/JDK mismatch: checked-out branch '" + detectedBranch + "' expects JDK "
				+ expectedJdkVersion + ", but the configured JDK home reports '"
				+ configuredJdkHomeVersionOrMissing + "'. Fix the machine-local JDK-home configuration "
				+ "(see config/jdk-home.local.properties) before running this tool.");
		this.detectedBranch = detectedBranch;
		this.configuredJdkHomeVersionOrMissing = configuredJdkHomeVersionOrMissing;
		this.expectedJdkVersion = expectedJdkVersion;
	}

	String detectedBranch() {
		return detectedBranch;
	}

	String configuredJdkHomeVersionOrMissing() {
		return configuredJdkHomeVersionOrMissing;
	}

	String expectedJdkVersion() {
		return expectedJdkVersion;
	}
}

/**
 * The git working tree wasn't clean at pre-flight (a prior run's leftover
 * state, most commonly an unresolved {@code COMMIT_FAILED} -- see
 * monitoring-design.md's reporting nuance, reflected in this message).
 */
final class DirtyWorkingTreeException extends EnvironmentCheckException {

	private final String gitStatusOutput;

	DirtyWorkingTreeException(String gitStatusOutput) {
		super("Working tree is not clean:" + System.lineSeparator() + gitStatusOutput
				+ System.lineSeparator()
				+ "This may be an earlier run's unresolved COMMIT_FAILED (see the report from that run) "
				+ "rather than a new problem -- check `git status`/`git diff` for an unexpected "
				+ "dependency-version change in pom.xml before resolving.");
		this.gitStatusOutput = gitStatusOutput;
	}

	String gitStatusOutput() {
		return gitStatusOutput;
	}
}
