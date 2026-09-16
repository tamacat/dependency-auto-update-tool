package org.tamacat.depupdate;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;

/**
 * Real {@link Validator}: business-rules.md BR-4's full procedure -- rewrite,
 * build, classify, revert-unless-green.
 */
public final class DefaultValidator implements Validator {

	private static final int DIAGNOSTIC_MAX_LINES = 40;
	private static final int DIAGNOSTIC_MAX_CHARS = 4000;

	private final Path pomXmlPath;
	private final Path workingDirectory;
	private final Path jdkHome;
	private final BuildRunner buildRunner;

	public DefaultValidator(Path pomXmlPath, Path workingDirectory, Path jdkHome) {
		this(pomXmlPath, workingDirectory, jdkHome, new ProcessBuildRunner());
	}

	public DefaultValidator(Path pomXmlPath, Path workingDirectory, Path jdkHome, BuildRunner buildRunner) {
		this.pomXmlPath = pomXmlPath;
		this.workingDirectory = workingDirectory;
		this.jdkHome = jdkHome;
		this.buildRunner = buildRunner;
	}

	@Override
	public ValidationResult validate(UpdateCandidate candidate) {
		String originalText;
		try {
			originalText = Files.readString(pomXmlPath, StandardCharsets.UTF_8);
		} catch (IOException e) {
			return new ValidationResult(candidate, ValidationOutcome.INFRA_FAILURE,
					"Could not read pom.xml: " + e.getMessage());
		}

		String modifiedText;
		try {
			modifiedText = PomRewriter.rewriteVersion(originalText, candidate.target(), candidate.candidateVersion());
		} catch (RuntimeException e) {
			return new ValidationResult(candidate, ValidationOutcome.INFRA_FAILURE,
					"Could not rewrite pom.xml for this candidate: " + e.getMessage());
		}

		try {
			Files.writeString(pomXmlPath, modifiedText, StandardCharsets.UTF_8);
		} catch (IOException e) {
			return new ValidationResult(candidate, ValidationOutcome.INFRA_FAILURE,
					"Could not write modified pom.xml: " + e.getMessage());
		}

		BuildResult buildResult;
		try {
			buildResult = buildRunner.run(workingDirectory, jdkHome);
		} catch (IOException | InterruptedException e) {
			if (e instanceof InterruptedException) {
				// Finding 6, code-generation review: restore the interrupt status
				// rather than swallowing it.
				Thread.currentThread().interrupt();
			}
			revertQuietly(originalText);
			return new ValidationResult(candidate, ValidationOutcome.INFRA_FAILURE,
					"Could not run mvn clean verify: " + e.getMessage());
		}

		ValidationOutcome outcome = ValidationOutcomeClassifier.classify(buildResult.exitCode(), buildResult.combinedOutput());
		if (outcome == ValidationOutcome.GREEN) {
			// Left in place for Applier, per BR-4.
			return new ValidationResult(candidate, ValidationOutcome.GREEN, "");
		}

		revertQuietly(originalText);
		return new ValidationResult(candidate, outcome, extractDiagnostic(buildResult.combinedOutput()));
	}

	private void revertQuietly(String originalText) {
		try {
			Files.writeString(pomXmlPath, originalText, StandardCharsets.UTF_8);
		} catch (IOException e) {
			System.err.println("WARNING: failed to revert pom.xml after a failed validation: " + e.getMessage());
		}
	}

	private static String extractDiagnostic(String combinedOutput) {
		if (combinedOutput == null || combinedOutput.isBlank()) {
			return "(mvn clean verify produced no output)";
		}
		String[] lines = combinedOutput.split("\\R");
		int from = Math.max(0, lines.length - DIAGNOSTIC_MAX_LINES);
		String excerpt = String.join(System.lineSeparator(), Arrays.copyOfRange(lines, from, lines.length));
		if (excerpt.length() > DIAGNOSTIC_MAX_CHARS) {
			excerpt = excerpt.substring(excerpt.length() - DIAGNOSTIC_MAX_CHARS);
		}
		return excerpt;
	}
}
