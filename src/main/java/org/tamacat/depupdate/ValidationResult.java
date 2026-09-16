package org.tamacat.depupdate;

import java.util.Objects;

/**
 * The result of {@code Validator.validate(candidate)}.
 *
 * @param candidate the candidate that was validated
 * @param outcome GREEN / INCOMPATIBLE / INFRA_FAILURE
 * @param diagnostic a short summary -- the relevant build/test failure
 *        excerpt for INCOMPATIBLE, the infra error for INFRA_FAILURE, or an
 *        empty string for GREEN
 */
public record ValidationResult(UpdateCandidate candidate, ValidationOutcome outcome, String diagnostic) {

	public ValidationResult {
		Objects.requireNonNull(candidate, "candidate");
		Objects.requireNonNull(outcome, "outcome");
		Objects.requireNonNull(diagnostic, "diagnostic");
	}
}
