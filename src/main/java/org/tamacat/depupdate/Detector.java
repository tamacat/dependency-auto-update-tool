package org.tamacat.depupdate;

import java.util.List;

/**
 * Establishes it is safe to run, and produces the list of candidate
 * dependency updates for this invocation (components.md).
 */
public interface Detector {

	/**
	 * Pre-flight: detects the current git branch, resolves that branch's
	 * configured JDK home (a machine-local, gitignored setting) and
	 * validates it reports the expected JDK major version -- NOT the tool's
	 * own running JVM, which targets a modern Java version independent of
	 * either branch (corrected at NFR Requirements, 2026-09-04T23:41:28Z) --
	 * then verifies the working tree is clean. Throws
	 * {@link EnvironmentCheckException} on any failure (fail-fast). Callers
	 * must route this exception to {@code Reporter.recordEnvironmentFailure()}
	 * before stopping.
	 */
	void validateEnvironment() throws EnvironmentCheckException;

	/**
	 * Parses {@code pom.xml}, groups its direct non-test-scope dependencies
	 * into {@link VersionTarget}s (business-rules.md BR-1a), and queries a
	 * Maven repository directly for available versions for each. A
	 * per-target repository-query failure (network/timeout) does NOT abort
	 * the run -- it is captured as its own {@link DetectionResult} so the
	 * remaining targets still get checked (ADR-10). Each successfully-queried
	 * target is filtered by its effective version-range policy (BR-1a),
	 * defaulting to {@code PATCH_ONLY}.
	 *
	 * @return one {@link DetectionResult} per in-scope target, always --
	 *         never throws for a per-target failure
	 */
	List<DetectionResult> findCandidates();
}
