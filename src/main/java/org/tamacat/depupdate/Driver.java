package org.tamacat.depupdate;

import java.io.IOException;
import java.nio.file.Path;
import java.util.List;

/**
 * Wires the four components and runs one invocation end-to-end, per
 * business-logic-model.md's End-to-End Workflow: pre-flight -&gt; detection
 * -&gt; per-candidate validate/apply loop (with the {@code COMMIT_FAILED}-stops-
 * the-run early exit) -&gt; finalization -&gt; exit code.
 */
public final class Driver {

	/** CLI entry point: parses arguments, builds real components, and runs. */
	public int run(String[] args) {
		if (args.length < 1) {
			System.err.println("Usage: java -jar dependency-auto-update-tool.jar <repoRoot> "
					+ "[toolConfigPath] [jdkHomeConfigPath]");
			System.err.println("  repoRoot           path to the tamacat-httpd checkout to operate on (required)");
			System.err.println("  toolConfigPath     default: config/tool.properties (relative to CWD)");
			System.err.println("  jdkHomeConfigPath  default: config/jdk-home.local.properties (relative to CWD)");
			return 2;
		}

		Path repoRoot = Path.of(args[0]).toAbsolutePath().normalize();
		Path toolConfigPath = args.length > 1 ? Path.of(args[1]) : Path.of("config", "tool.properties");
		Path jdkHomeConfigPath = args.length > 2 ? Path.of(args[2]) : Path.of("config", "jdk-home.local.properties");

		ToolConfiguration configuration;
		try {
			configuration = ToolConfiguration.loadFromProperties(toolConfigPath);
		} catch (IOException e) {
			System.err.println("Failed to load tool configuration from " + toolConfigPath + ": " + e.getMessage());
			return 2;
		}

		JdkHomeResolver jdkHomeResolver;
		try {
			jdkHomeResolver = PropertiesJdkHomeResolver.load(jdkHomeConfigPath);
		} catch (IOException e) {
			System.err.println(
					"Failed to load JDK-home configuration from " + jdkHomeConfigPath + ": " + e.getMessage());
			return 2;
		}

		Reporter reporter = new DefaultReporter(System.out, configuration.reportDirectory());
		GitClient gitClient = new ProcessGitClient(repoRoot);
		VersionRepositoryClient repositoryClient = new MavenCentralVersionRepositoryClient(
				configuration.repositoryBaseUrl(), configuration.repositoryQueryTimeout());
		JdkVersionChecker jdkVersionChecker = new ProcessJdkVersionChecker();
		Path pomXmlPath = repoRoot.resolve("pom.xml");

		DefaultDetector detector = new DefaultDetector(pomXmlPath, configuration, jdkHomeResolver, jdkVersionChecker,
				repositoryClient, gitClient);

		try {
			detector.validateEnvironment();
		} catch (EnvironmentCheckException e) {
			reporter.recordEnvironmentFailure(e);
			reporter.finish();
			return 1;
		}

		Validator validator = new DefaultValidator(pomXmlPath, repoRoot, detector.getResolvedJdkHome());
		Applier applier = new DefaultApplier(gitClient);

		List<DetectionResult> detectionResults = detector.findCandidates();
		return executeAfterPreflight(detectionResults, validator, applier, reporter, configuration.commitPushMode());
	}

	/**
	 * The orchestration core, testable independently of CLI parsing and real
	 * I/O: given detection results and already-wired Validator/Applier/Reporter,
	 * runs Step 3 (validate/apply loop) and Step 4 (finalization) of
	 * business-logic-model.md, and returns the process exit code.
	 */
	int executeAfterPreflight(List<DetectionResult> detectionResults, Validator validator, Applier applier,
			Reporter reporter, CommitPushMode commitPushMode) {
		boolean needsAttention = false;

		for (DetectionResult r : detectionResults) {
			reporter.recordDetection(r);
			if (r instanceof QueryFailed) {
				needsAttention = true;
			}
		}

		for (DetectionResult r : detectionResults) {
			if (!(r instanceof CandidateFound cf)) {
				continue;
			}
			UpdateCandidate candidate = cf.candidate();
			ValidationResult validationResult = validator.validate(candidate);

			if (validationResult.outcome() == ValidationOutcome.GREEN) {
				ApplyResult applyResult = applier.apply(validationResult, commitPushMode);
				reporter.record(candidate, validationResult, applyResult);
				if (applyResult instanceof CommitFailed) {
					// NFR Design Change Request (audit-logged 2026-09-05T00:23:31Z): stop
					// the run rather than risk a later candidate's successful commit
					// silently bundling this candidate's still-uncommitted pom.xml change.
					needsAttention = true;
					break;
				}
				if (applyResult instanceof CommittedPushFailed) {
					needsAttention = true;
				}
			} else {
				reporter.record(candidate, validationResult, null);
				needsAttention = true;
			}
		}

		reporter.finish();
		return needsAttention ? 1 : 0;
	}
}
