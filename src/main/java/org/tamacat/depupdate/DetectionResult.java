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
 * Reported, not validated. {@code security} says whether the current version
 * has known vulnerabilities that no allowed version fixes. {@code javaExclusion}
 * says which newer versions were left out because they need a newer Java than
 * the branch's JDK ({@code null} when none were).
 */
record NoUpdateAvailable(VersionTarget target, SecurityStatus security, JavaExclusion javaExclusion)
		implements DetectionResult {

	NoUpdateAvailable(VersionTarget target) {
		this(target, SecurityStatus.NONE);
	}

	NoUpdateAvailable(VersionTarget target, SecurityStatus security) {
		this(target, security, null);
	}

	NoUpdateAvailable {
		Objects.requireNonNull(target, "target");
		Objects.requireNonNull(security, "security");
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
