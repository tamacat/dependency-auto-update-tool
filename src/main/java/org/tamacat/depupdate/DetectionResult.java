package org.tamacat.depupdate;

import java.util.Objects;

/**
 * A closed set of three variants (Application Design ADR-10), one produced
 * per in-scope {@link VersionTarget} by {@code Detector.findCandidates()}.
 */
public sealed interface DetectionResult permits CandidateFound, NoUpdateAvailable, QueryFailed {

	/** The target this result concerns. */
	VersionTarget target();
}

/** Proceeds to Validator. */
record CandidateFound(UpdateCandidate candidate) implements DetectionResult {

	CandidateFound {
		Objects.requireNonNull(candidate, "candidate");
	}

	@Override
	public VersionTarget target() {
		return candidate.target();
	}
}

/**
 * Nothing in-policy is newer for this target. For a
 * {@code PropertyIndirectedTarget}, this includes the case where a newer
 * version exists for some but not all members (business-rules.md BR-1a).
 * Reported, not validated.
 */
record NoUpdateAvailable(VersionTarget target) implements DetectionResult {

	NoUpdateAvailable {
		Objects.requireNonNull(target, "target");
	}
}

/**
 * A repository query for one of the target's member coordinates failed.
 * Reported, not validated; does not abort the run (ADR-10).
 */
record QueryFailed(VersionTarget target, String diagnostic) implements DetectionResult {

	QueryFailed {
		Objects.requireNonNull(target, "target");
		Objects.requireNonNull(diagnostic, "diagnostic");
	}
}
