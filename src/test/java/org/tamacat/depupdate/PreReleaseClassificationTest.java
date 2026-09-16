package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

/** business-rules.md BR-2: pre-release / stable classification. */
class PreReleaseClassificationTest {

	@Test
	void plainRelease_isStable() {
		assertEquals(VersionClassification.STABLE, PreReleaseClassifier.classify("5.5.0"));
	}

	@Test
	void snapshot_isPreRelease() {
		assertEquals(VersionClassification.PRE_RELEASE, PreReleaseClassifier.classify("5.5.0-SNAPSHOT"));
	}

	@Test
	void alpha_isPreRelease() {
		assertEquals(VersionClassification.PRE_RELEASE, PreReleaseClassifier.classify("1.0.0-alpha1"));
	}

	@Test
	void beta_isPreRelease_caseInsensitive() {
		// The real pom.xml's own httpcore5 dependency version shape.
		assertEquals(VersionClassification.PRE_RELEASE, PreReleaseClassifier.classify("5.5-beta2"));
		assertEquals(VersionClassification.PRE_RELEASE, PreReleaseClassifier.classify("5.5-BETA2"));
	}

	@Test
	void releaseCandidateWithDash_isPreRelease() {
		assertEquals(VersionClassification.PRE_RELEASE, PreReleaseClassifier.classify("2.0.0-RC1"));
	}

	@Test
	void releaseCandidateWithDot_isPreRelease() {
		assertEquals(VersionClassification.PRE_RELEASE, PreReleaseClassifier.classify("2.0.0.RC1"));
	}

	@Test
	void mavenStyleMilestone_isPreRelease() {
		assertEquals(VersionClassification.PRE_RELEASE, PreReleaseClassifier.classify("1.0.0-M1"));
	}

	@Test
	void releaseQualifier_isNotMisclassifiedAsReleaseCandidate() {
		// The real pom.xml's own thymeleaf dependency: "3.1.5.RELEASE" must not
		// false-positive-match the "rc" pattern.
		assertEquals(VersionClassification.STABLE, PreReleaseClassifier.classify("3.1.5.RELEASE"));
	}
}
