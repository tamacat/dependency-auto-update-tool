package org.tamacat.depupdate;

/**
 * The classification of a version bump between a current and a candidate
 * version (business-rules.md BR-1). Ordinal order matters: it lines up with
 * {@link VersionRangePolicy}'s ordinal order so that
 * {@code policy.ordinal() >= bump.ordinal()} means "this bump is within the
 * policy's allowed range" -- see {@link VersionRangePolicy#allows(VersionBump)}.
 */
public enum VersionBump {
	PATCH,
	MINOR,
	MAJOR
}
