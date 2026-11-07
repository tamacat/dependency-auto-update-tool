package org.tamacat.depupdate;

import java.util.Optional;

/**
 * How far this tool may go beyond a dependency's normal
 * {@link VersionRangePolicy} to reach a version that fixes known
 * vulnerabilities in the current one (configured as {@code securityFixPolicy}).
 *
 * <ul>
 * <li>{@code OFF}: never look up vulnerabilities (no request to the
 * vulnerability database at all) -- the tool behaves exactly as before.</li>
 * <li>{@code PATCH_ONLY} / {@code MINOR} / {@code MAJOR}: when the current
 * version has known vulnerabilities and the normal policy's best candidate
 * does not fix them, a candidate up to this bump size may be chosen instead.
 * The normal policy is never narrowed: the ceiling is whichever of the two
 * allows more. Security-fix candidates are also validated first.</li>
 * </ul>
 *
 * <p>The candidate is still validated by the real build ({@code mvn clean
 * verify}) and, by default, only committed locally -- a larger bump is
 * proposed, never pushed unreviewed unless {@code FULL_LOOP_AUTO_PUSH} is set.
 */
public enum SecurityFixPolicy {
	OFF,
	PATCH_ONLY,
	MINOR,
	MAJOR;

	/** The default: allow up to a minor bump to fix a known vulnerability. */
	public static final SecurityFixPolicy DEFAULT = MINOR;

	/** The bump ceiling for security fixes, or empty when vulnerabilities are not looked up. */
	public Optional<VersionRangePolicy> ceiling() {
		return switch (this) {
			case OFF -> Optional.empty();
			case PATCH_ONLY -> Optional.of(VersionRangePolicy.PATCH_ONLY);
			case MINOR -> Optional.of(VersionRangePolicy.MINOR);
			case MAJOR -> Optional.of(VersionRangePolicy.MAJOR);
		};
	}
}
