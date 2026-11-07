package org.tamacat.depupdate;

import java.util.Objects;
import java.util.Set;

/**
 * What Detector learned about known vulnerabilities for one target. Carried
 * on {@link UpdateCandidate} and {@code NoUpdateAvailable} so Reporter and the
 * commit message can say what an update fixes.
 *
 * @param vulnerable vulnerability IDs (OSV, e.g. {@code GHSA-...}) affecting the current version
 * @param fixed the subset of {@code vulnerable} that the candidate no longer has
 * @param remaining the subset of {@code vulnerable} that the candidate still has
 *        (or, without a candidate, all of {@code vulnerable})
 * @param escalated whether the candidate is outside the dependency's normal
 *        {@link VersionRangePolicy} and was chosen only because it fixes vulnerabilities
 * @param diagnostic why vulnerabilities could not be looked up, or {@code null}
 */
public record SecurityStatus(Set<String> vulnerable, Set<String> fixed, Set<String> remaining, boolean escalated,
		String diagnostic) {

	/** Nothing known: not looked up, or no vulnerabilities. */
	public static final SecurityStatus NONE = new SecurityStatus(Set.of(), Set.of(), Set.of(), false, null);

	public SecurityStatus {
		vulnerable = Set.copyOf(Objects.requireNonNull(vulnerable, "vulnerable"));
		fixed = Set.copyOf(Objects.requireNonNull(fixed, "fixed"));
		remaining = Set.copyOf(Objects.requireNonNull(remaining, "remaining"));
	}

	/** A lookup that failed; the target is handled as if nothing were known. */
	public static SecurityStatus lookupFailed(String diagnostic) {
		return new SecurityStatus(Set.of(), Set.of(), Set.of(), false, diagnostic);
	}

	/** Whether the candidate fixes at least one known vulnerability. */
	public boolean fixesSomething() {
		return !fixed.isEmpty();
	}
}
