package org.tamacat.depupdate;

/**
 * Produces the record of what happened, for every target Detector enumerated
 * (FR-4.1, components.md).
 */
public interface Reporter {

	/**
	 * Records one target's detection-time outcome (CANDIDATE_FOUND /
	 * NO_UPDATE_AVAILABLE / QUERY_FAILED -- ADR-10), before validation
	 * happens.
	 */
	void recordDetection(DetectionResult detectionResult);

	/**
	 * Records one candidate's full downstream outcome -- the
	 * {@link ValidationResult}, and the {@link ApplyResult} (one of its four
	 * variants) if Validator reached GREEN, or {@code null} otherwise -- for
	 * the final report.
	 */
	void record(UpdateCandidate candidate, ValidationResult validationResult, ApplyResult applyResult);

	/**
	 * Records a pre-flight environment failure (branch/JDK mismatch or dirty
	 * working tree) that stopped the run before any candidates were found.
	 */
	void recordEnvironmentFailure(EnvironmentCheckException failure);

	/**
	 * Emits the final report covering everything recorded during this run.
	 */
	void finish();
}
