package org.tamacat.depupdate;

/**
 * The outcome of running a candidate through Validator's real build/test
 * validation (FR-2.2, business-rules.md BR-5).
 */
public enum ValidationOutcome {
	/** Safe to apply. */
	GREEN,
	/** The build/test genuinely failed with this candidate applied. */
	INCOMPATIBLE,
	/** The validation run itself failed for reasons unrelated to the candidate. */
	INFRA_FAILURE
}
