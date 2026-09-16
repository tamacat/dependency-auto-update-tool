package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

/**
 * business-rules.md BR-5: pure text-signature classification -- no
 * filesystem dependency, so this is fast and fully mockable.
 */
class ValidationOutcomeClassificationTest {

	@Test
	void zeroExitCode_isGreen_regardlessOfOutput() {
		assertEquals(ValidationOutcome.GREEN, ValidationOutcomeClassifier.classify(0, "anything at all"));
	}

	@Test
	void nonZeroExit_dependencyResolutionFailure_isInfraFailure() {
		String output = "[ERROR] Failed to execute goal ...\n"
				+ "Could not resolve dependencies for project org.tamacat:foo:jar:1.0\n";
		assertEquals(ValidationOutcome.INFRA_FAILURE, ValidationOutcomeClassifier.classify(1, output));
	}

	@Test
	void nonZeroExit_artifactTransferFailure_isInfraFailure() {
		String output = "Could not transfer artifact org.example:foo:jar:1.0 from/to central";
		assertEquals(ValidationOutcome.INFRA_FAILURE, ValidationOutcomeClassifier.classify(1, output));
	}

	@Test
	void nonZeroExit_connectionRefused_isInfraFailure_caseInsensitive() {
		String output = "java.net.ConnectException: CONNECTION REFUSED";
		assertEquals(ValidationOutcome.INFRA_FAILURE, ValidationOutcomeClassifier.classify(1, output));
	}

	@Test
	void nonZeroExit_unknownHostException_isInfraFailure() {
		String output = "Caused by: java.net.UnknownHostException: repo1.maven.org";
		assertEquals(ValidationOutcome.INFRA_FAILURE, ValidationOutcomeClassifier.classify(1, output));
	}

	@Test
	void nonZeroExit_metadataTransferFailure_isInfraFailure() {
		String output = "Could not transfer metadata org.example:foo/maven-metadata.xml from/to central";
		assertEquals(ValidationOutcome.INFRA_FAILURE, ValidationOutcomeClassifier.classify(1, output));
	}

	@Test
	void nonZeroExit_connectionTimedOut_isInfraFailure() {
		String output = "java.net.SocketTimeoutException: Connection timed out";
		assertEquals(ValidationOutcome.INFRA_FAILURE, ValidationOutcomeClassifier.classify(1, output));
	}

	@Test
	void nonZeroExit_nonResolvableImportPom_isInfraFailure() {
		String output = "[FATAL] Non-resolvable import POM: org.example:bom:1.0 was not found";
		assertEquals(ValidationOutcome.INFRA_FAILURE, ValidationOutcomeClassifier.classify(1, output));
	}

	@Test
	void nonZeroExit_pkixPathBuildingFailed_isInfraFailure() {
		String output = "sun.security.provider.certpath.SunCertPathBuilderException: "
				+ "PKIX path building failed: unable to find valid certification path";
		assertEquals(ValidationOutcome.INFRA_FAILURE, ValidationOutcomeClassifier.classify(1, output));
	}

	@Test
	void nonZeroExit_genuineTestFailure_isIncompatible() {
		String output = "Tests run: 12, Failures: 1, Errors: 0, Skipped: 0\n"
				+ "[ERROR] FooTest.testBar:42 expected:<1> but was:<2>\n"
				+ "[ERROR] BUILD FAILURE";
		assertEquals(ValidationOutcome.INCOMPATIBLE, ValidationOutcomeClassifier.classify(1, output));
	}

	@Test
	void nonZeroExit_compileTimeBreak_neverReachedTestPhase_isStillIncompatible() {
		// BR-5: a compile-time break "never reached the test phase at all" is
		// still INCOMPATIBLE, not INFRA_FAILURE -- both mean "this version
		// doesn't work with this codebase."
		String output = "[ERROR] COMPILATION ERROR :\n"
				+ "[ERROR] cannot find symbol\n"
				+ "[ERROR]   symbol:   method removedMethod()\n"
				+ "[ERROR] BUILD FAILURE";
		assertEquals(ValidationOutcome.INCOMPATIBLE, ValidationOutcomeClassifier.classify(1, output));
	}

	@Test
	void nonZeroExit_ambiguousOutputWithNoRecognizedSignature_defaultsToIncompatible() {
		// BR-5's conservative default: an unrecognized non-zero-exit case never
		// gets the benefit of the doubt as "just an infra hiccup."
		assertEquals(ValidationOutcome.INCOMPATIBLE,
				ValidationOutcomeClassifier.classify(1, "something went wrong, no idea what"));
	}
}
