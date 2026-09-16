package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

/**
 * business-rules.md BR-1: version-bump classification (patch / minor /
 * major).
 */
class VersionBumpClassificationTest {

	@Test
	void firstSegmentDiffers_isMajor() {
		assertEquals(VersionBump.MAJOR, VersionBumpClassifier.classify("1.2.3", "2.0.0"));
	}

	@Test
	void firstEqualSecondDiffers_isMinor() {
		assertEquals(VersionBump.MINOR, VersionBumpClassifier.classify("1.2.3", "1.3.0"));
	}

	@Test
	void firstAndSecondEqualThirdDiffers_isPatch() {
		assertEquals(VersionBump.PATCH, VersionBumpClassifier.classify("1.2.3", "1.2.4"));
	}

	@Test
	void qualifierOnlyChange_isPatch() {
		// "Anything else differs (3rd+ segment, qualifier, or nothing at all)" -> PATCH.
		assertEquals(VersionBump.PATCH, VersionBumpClassifier.classify("1.2.3", "1.2.3-RC1"));
	}

	@Test
	void realHttpcore5BetaBump_isPatch() {
		// The real pom.xml's own httpcore5 dependency: 5.5-beta2 -> a hypothetical
		// later 5.5-beta3 patch differs only after the first two numeric segments.
		assertEquals(VersionBump.PATCH, VersionBumpClassifier.classify("5.5-beta2", "5.5-beta3"));
	}

	@Test
	void fewerNumericSegmentsInCurrent_secondAbsentVsPresent_isMinor() {
		// Explicit §12a review edge case: one version has fewer numeric segments
		// than the other. "2" vs "2.1" -> 1st equal (2==2), 2nd absent in "2" but
		// present ("1") in "2.1" -> MINOR, per BR-1's own "absent in one, present
		// in the other" clause.
		assertEquals(VersionBump.MINOR, VersionBumpClassifier.classify("2", "2.1"));
	}

	@Test
	void fewerNumericSegmentsInCandidate_secondPresentVsAbsent_isMinor() {
		assertEquals(VersionBump.MINOR, VersionBumpClassifier.classify("2.1", "2"));
	}

	@Test
	void singleNumericSegmentBothSides_firstDiffers_isMajor() {
		assertEquals(VersionBump.MAJOR, VersionBumpClassifier.classify("2", "3"));
	}

	@Test
	void identicalVersions_isPatch() {
		// Not itself a real "update" (Detector filters non-newer versions out
		// before classification is even consulted -- see VersionComparator), but
		// BR-1's own rule text says equal-tuple-and-nothing-else-differs is PATCH.
		assertEquals(VersionBump.PATCH, VersionBumpClassifier.classify("1.2.3", "1.2.3"));
	}
}
