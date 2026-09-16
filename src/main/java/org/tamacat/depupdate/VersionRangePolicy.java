package org.tamacat.depupdate;

import java.util.Collection;
import java.util.Comparator;

/**
 * Per-{@link DependencyCoordinate} configurable policy for how large a
 * version bump is eligible to become a candidate (Application Design Q7: C,
 * ADR-8). {@code PATCH_ONLY} is the default when a dependency has no
 * explicit configuration entry.
 *
 * <p>Ordinal order is deliberate: it matches {@link VersionBump}'s ordinal
 * order (PATCH=0, MINOR=1, MAJOR=2), so {@link #allows(VersionBump)} is a
 * simple ordinal comparison, and natural enum order is already "most
 * restrictive to least restrictive" for {@link #mostRestrictive(Collection)}.
 */
public enum VersionRangePolicy {
	PATCH_ONLY,
	MINOR,
	MAJOR;

	/** Whether a bump of this classification is within this policy's allowed range. */
	public boolean allows(VersionBump bump) {
		return bump.ordinal() <= this.ordinal();
	}

	/** Whether this policy allows at least as much as {@code other} (ordinal comparison). */
	public boolean isAtLeastAsPermissiveAs(VersionRangePolicy other) {
		return this.ordinal() >= other.ordinal();
	}

	/**
	 * BR-1a step 2: for a {@code PropertyIndirectedTarget}, the effective
	 * policy is the most restrictive (most conservative) policy among all of
	 * its members' individually-configured (or defaulted) policies -- the
	 * group moves only as far as its most conservative member allows.
	 */
	public static VersionRangePolicy mostRestrictive(Collection<VersionRangePolicy> policies) {
		return policies.stream().min(Comparator.naturalOrder()).orElse(PATCH_ONLY);
	}
}
