package org.tamacat.depupdate;

import java.util.List;
import java.util.Locale;
import java.util.Optional;

/**
 * business-rules.md BR-5 (revised during Functional Design's fix round):
 * classify a {@code mvn clean verify} run using <strong>only</strong> its exit
 * code and a case-insensitive substring match against the captured combined
 * stdout/stderr -- no filesystem inspection.
 *
 * <p>A zero exit code is {@code GREEN}, unless the output shows that a class
 * could not be loaded because it was built for a newer Java (see
 * {@link #runtimeIncompatibility(String)}): a library loaded only at run time
 * can fail that way while every test still passes, so that is
 * {@code INCOMPATIBLE}.
 *
 * <p>A non-zero exit code is {@code INFRA_FAILURE} when the output matches a
 * known infrastructure signature. Any other case (including a genuine test
 * failure or a compile-time break) defaults to {@code INCOMPATIBLE}, the
 * conservative choice: neither outcome is ever applied, but mislabeling a real
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

	/**
	 * A class built for a newer Java than the one running it. The SLF4J line
	 * is what SLF4J 2 prints when its provider (logback, for example) cannot be
	 * loaded; it then falls back to a logger that drops everything, without
	 * failing anything and without printing the underlying error.
	 */
	private static final List<String> RUNTIME_INCOMPATIBILITY_SIGNATURES = List.of(
			"unsupportedclassversionerror",
			"has been compiled by a more recent version of the java runtime",
			"slf4j(e): a service provider failed to instantiate");

	/** Lines of output kept after a matching line, for the diagnostic. */
	private static final int RUNTIME_INCOMPATIBILITY_CONTEXT_LINES = 3;

	private ValidationOutcomeClassifier() {
	}

	public static ValidationOutcome classify(int exitCode, String combinedOutput) {
		if (exitCode == 0) {
			return runtimeIncompatibility(combinedOutput).isPresent() ? ValidationOutcome.INCOMPATIBLE
					: ValidationOutcome.GREEN;
		}
		String lower = combinedOutput == null ? "" : combinedOutput.toLowerCase(Locale.ROOT);
		for (String signature : INFRA_FAILURE_SIGNATURES) {
			if (lower.contains(signature)) {
				return ValidationOutcome.INFRA_FAILURE;
			}
		}
		return ValidationOutcome.INCOMPATIBLE;
	}

	/**
	 * The first output line showing a class that could not be loaded because
	 * it was built for a newer Java, with the few lines after it; empty when
	 * there is none.
	 */
	public static Optional<String> runtimeIncompatibility(String combinedOutput) {
		if (combinedOutput == null || combinedOutput.isEmpty()) {
			return Optional.empty();
		}
		String[] lines = combinedOutput.split("\\R");
		for (int i = 0; i < lines.length; i++) {
			String lower = lines[i].toLowerCase(Locale.ROOT);
			for (String signature : RUNTIME_INCOMPATIBILITY_SIGNATURES) {
				if (lower.contains(signature)) {
					int to = Math.min(lines.length, i + 1 + RUNTIME_INCOMPATIBILITY_CONTEXT_LINES);
					return Optional.of(String.join(System.lineSeparator(), List.of(lines).subList(i, to)));
				}
			}
		}
		return Optional.empty();
	}
}
