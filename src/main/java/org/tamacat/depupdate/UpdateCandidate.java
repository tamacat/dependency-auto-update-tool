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
 * @param security known vulnerabilities in the current version and which of
 *        them this candidate fixes ({@link SecurityStatus#NONE} when not looked up)
 */
public record UpdateCandidate(VersionTarget target, String currentVersion, String candidateVersion,
		VersionClassification classification, SecurityStatus security) {

	/** A candidate with no vulnerability information. */
	public UpdateCandidate(VersionTarget target, String currentVersion, String candidateVersion,
			VersionClassification classification) {
		this(target, currentVersion, candidateVersion, classification, SecurityStatus.NONE);
	}

	public UpdateCandidate {
		Objects.requireNonNull(target, "target");
		Objects.requireNonNull(classification, "classification");
		Objects.requireNonNull(security, "security");
		if (currentVersion == null || currentVersion.isBlank()) {
			throw new IllegalArgumentException("currentVersion must not be blank");
		}
		if (candidateVersion == null || candidateVersion.isBlank()) {
			throw new IllegalArgumentException("candidateVersion must not be blank");
		}
	}
}
