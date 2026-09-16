package org.tamacat.depupdate;

import java.util.List;
import java.util.Locale;

/**
 * business-rules.md BR-5 (revised during Functional Design's fix round):
 * given a non-zero {@code mvn clean verify} exit code, classify using
 * <strong>only</strong> a case-insensitive substring match against the
 * captured combined stdout/stderr -- no filesystem inspection. Any
 * unrecognized non-zero-exit case (including a genuine test failure or a
 * compile-time break) defaults to {@code INCOMPATIBLE}, the conservative
 * choice: neither outcome is ever applied, but mislabeling a real
 * incompatibility as a retriable infra hiccup would be actively misleading.
 */
public final class ValidationOutcomeClassifier {

	private static final List<String> INFRA_FAILURE_SIGNATURES = List.of(
			"could not resolve dependencies",
			"could not transfer artifact",
			"could not transfer metadata",
			"connection timed out",
			"connection refused",
			"unknownhostexception",
			"non-resolvable import pom",
			"pkix path building failed");

	private ValidationOutcomeClassifier() {
	}

	public static ValidationOutcome classify(int exitCode, String combinedOutput) {
		if (exitCode == 0) {
			return ValidationOutcome.GREEN;
		}
		String lower = combinedOutput == null ? "" : combinedOutput.toLowerCase(Locale.ROOT);
		for (String signature : INFRA_FAILURE_SIGNATURES) {
			if (lower.contains(signature)) {
				return ValidationOutcome.INFRA_FAILURE;
			}
		}
		return ValidationOutcome.INCOMPATIBLE;
	}
}
