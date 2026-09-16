package org.tamacat.depupdate;

import java.util.Objects;

/**
 * A candidate update Detector found for one {@link VersionTarget} (FR-1.2).
 * Created inside {@code DetectionResult.CandidateFound}; consumed by
 * Validator; carried through to Reporter regardless of downstream outcome.
 * Never mutated after creation.
 *
 * @param target the target this candidate applies to
 * @param currentVersion the literal current value (for a
 *        {@code PropertyIndirectedTarget}, the property's current resolved
 *        value) -- duplicated from {@code target.currentVersion()} for
 *        convenience at every call site that only has the candidate
 * @param candidateVersion the version Detector proposes to move to
 * @param classification stable vs. pre-release (business-rules.md BR-2)
 */
public record UpdateCandidate(VersionTarget target, String currentVersion, String candidateVersion,
		VersionClassification classification) {

	public UpdateCandidate {
		Objects.requireNonNull(target, "target");
		Objects.requireNonNull(classification, "classification");
		if (currentVersion == null || currentVersion.isBlank()) {
			throw new IllegalArgumentException("currentVersion must not be blank");
		}
		if (candidateVersion == null || candidateVersion.isBlank()) {
			throw new IllegalArgumentException("candidateVersion must not be blank");
		}
	}
}
