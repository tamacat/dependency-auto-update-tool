package org.tamacat.depupdate;

/**
 * Stable/release vs. pre-release/snapshot classification of a candidate
 * version (FR-1.2, business-rules.md BR-2). Both classifications still go
 * through validation and the version-range-policy filter -- this only
 * changes how Reporter labels the outcome, never whether it is attempted.
 */
public enum VersionClassification {
	STABLE,
	PRE_RELEASE
}
