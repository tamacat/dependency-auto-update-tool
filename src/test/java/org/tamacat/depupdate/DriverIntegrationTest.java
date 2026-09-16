package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

/**
 * Integration stub (key boundary): drives Detector -&gt; Validator -&gt;
 * Applier -&gt; Reporter against a small fixture {@code pom.xml} (both a
 * plain-dependency case and a property-indirected case), with the Maven
 * repository query and {@code mvn clean verify} subprocess faked, asserting
 * the {@code COMMIT_FAILED}-stops-remaining-candidates behavior end-to-end
 * (the exact scenario NFR Design's Change Request, audit-logged
 * 2026-09-05T00:23:31Z, fixed).
 */
class DriverIntegrationTest {

	private static final DependencyCoordinate WIDGET = new DependencyCoordinate("org.example", "widget");
	private static final DependencyCoordinate TOMCAT_CORE = new DependencyCoordinate("org.example", "tomcat-embed-core");
	private static final DependencyCoordinate TOMCAT_JASPER = new DependencyCoordinate("org.example", "tomcat-embed-jasper");

	private static final String FIXTURE_POM = "<project>"
			+ "<properties><tomcat.version>1.0.0</tomcat.version></properties>"
			+ "<dependencies>"
			+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId><version>1.0.0</version></dependency>"
			+ "<dependency><groupId>org.example</groupId><artifactId>tomcat-embed-core</artifactId><version>${tomcat.version}</version></dependency>"
			+ "<dependency><groupId>org.example</groupId><artifactId>tomcat-embed-jasper</artifactId><version>${tomcat.version}</version></dependency>"
			+ "</dependencies></project>";

	@Test
	void commitFailedOnFirstCandidate_stopsTheRun_secondCandidateNeverValidatedOrApplied(@TempDir Path repoRoot)
			throws Exception {
		Path pomPath = repoRoot.resolve("pom.xml");
		Files.writeString(pomPath, FIXTURE_POM);

		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient()
				.withVersions(WIDGET, "1.0.1")
				.withVersions(TOMCAT_CORE, "1.0.1")
				.withVersions(TOMCAT_JASPER, "1.0.1");
		FakeGitClient git = new FakeGitClient();
		git.branch = "master";
		FakeBuildRunner buildRunner = new FakeBuildRunner().defaultingTo(new BuildResult(0, "BUILD SUCCESS"));

		DefaultDetector detector = new DefaultDetector(pomPath, ToolConfiguration.defaults(),
				new FakeJdkHomeResolver(), FakeJdkVersionChecker.returning(8), repo, git);
		List<DetectionResult> detectionResults = detector.findCandidates();
		assertEquals(2, detectionResults.size(), "widget (direct) + one grouped tomcat.version target");
		assertInstanceOf(CandidateFound.class, detectionResults.get(0));
		assertInstanceOf(CandidateFound.class, detectionResults.get(1));

		DefaultValidator validator = new DefaultValidator(pomPath, repoRoot, Path.of("unused-jdk-home"), buildRunner);
		// The FIRST commit attempt (for widget, processed first) fails.
		git.enqueueCommitOutcome(new CommitOutcome(false, null, "pre-commit hook rejected"));
		DefaultApplier applier = new DefaultApplier(git);
		RecordingReporter reporter = new RecordingReporter();

		int exitCode = new Driver().executeAfterPreflight(detectionResults, validator, applier, reporter,
				CommitPushMode.LOCAL_COMMIT_MANUAL_APPROVAL);

		assertEquals(1, exitCode, "a COMMIT_FAILED run must exit non-zero");
		assertEquals(2, reporter.detections.size(), "both detections are still reported up front");
		assertEquals(1, reporter.results.size(),
				"only the FIRST candidate's result is recorded -- the second is never reached");
		assertInstanceOf(CommitFailed.class, reporter.results.get(0).applyResult());
		assertEquals(1, buildRunner.runCount,
				"Validator.validate() must never be called for the second candidate once the run stops");
		assertEquals(1, git.commitCallCount);
		assertTrue(reporter.finished);

		// pom.xml reflects the first (widget) candidate's GREEN-validated change,
		// left in place per ADR-7 -- no revert on COMMIT_FAILED -- while the
		// second (tomcat.version) target was never even attempted.
		String finalPom = Files.readString(pomPath);
		assertTrue(finalPom.contains("<version>1.0.1</version>"), "widget's validated version change is left in place");
		assertTrue(finalPom.contains("<tomcat.version>1.0.0</tomcat.version>"),
				"tomcat.version property must be untouched -- its candidate was never reached");
	}

	@Test
	void allCandidatesGreenAndCommittedSuccessfully_bothCommitted_exitsZero(@TempDir Path repoRoot) throws Exception {
		Path pomPath = repoRoot.resolve("pom.xml");
		Files.writeString(pomPath, FIXTURE_POM);

		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient()
				.withVersions(WIDGET, "1.0.1")
				.withVersions(TOMCAT_CORE, "1.0.1")
				.withVersions(TOMCAT_JASPER, "1.0.1");
		FakeGitClient git = new FakeGitClient();
		git.branch = "master";
		FakeBuildRunner buildRunner = new FakeBuildRunner().defaultingTo(new BuildResult(0, "BUILD SUCCESS"));

		DefaultDetector detector = new DefaultDetector(pomPath, ToolConfiguration.defaults(),
				new FakeJdkHomeResolver(), FakeJdkVersionChecker.returning(8), repo, git);
		List<DetectionResult> detectionResults = detector.findCandidates();

		DefaultValidator validator = new DefaultValidator(pomPath, repoRoot, Path.of("unused-jdk-home"), buildRunner);
		DefaultApplier applier = new DefaultApplier(git);
		RecordingReporter reporter = new RecordingReporter();

		int exitCode = new Driver().executeAfterPreflight(detectionResults, validator, applier, reporter,
				CommitPushMode.LOCAL_COMMIT_MANUAL_APPROVAL);

		assertEquals(0, exitCode, "every target applied or up to date -> clean run");
		assertEquals(2, reporter.results.size(), "both candidates reached and recorded");
		assertEquals(2, buildRunner.runCount);
		assertEquals(2, git.commitCallCount);
		for (RecordingReporter.RecordedResult result : reporter.results) {
			assertInstanceOf(CommittedNoPushAttempted.class, result.applyResult());
		}
	}

	@Test
	void incompatibleCandidate_doesNotAbortRun_remainingCandidateStillProcessed(@TempDir Path repoRoot)
			throws Exception {
		Path pomPath = repoRoot.resolve("pom.xml");
		Files.writeString(pomPath, FIXTURE_POM);

		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient()
				.withVersions(WIDGET, "1.0.1")
				.withVersions(TOMCAT_CORE, "1.0.1")
				.withVersions(TOMCAT_JASPER, "1.0.1");
		FakeGitClient git = new FakeGitClient();
		git.branch = "master";
		// First validate() call (widget) fails the build; second (tomcat group) is GREEN.
		FakeBuildRunner buildRunner = new FakeBuildRunner()
				.enqueue(new BuildResult(1, "[ERROR] Tests run: 1, Failures: 1"))
				.defaultingTo(new BuildResult(0, "BUILD SUCCESS"));

		DefaultDetector detector = new DefaultDetector(pomPath, ToolConfiguration.defaults(),
				new FakeJdkHomeResolver(), FakeJdkVersionChecker.returning(8), repo, git);
		List<DetectionResult> detectionResults = detector.findCandidates();

		DefaultValidator validator = new DefaultValidator(pomPath, repoRoot, Path.of("unused-jdk-home"), buildRunner);
		DefaultApplier applier = new DefaultApplier(git);
		RecordingReporter reporter = new RecordingReporter();

		int exitCode = new Driver().executeAfterPreflight(detectionResults, validator, applier, reporter,
				CommitPushMode.LOCAL_COMMIT_MANUAL_APPROVAL);

		assertEquals(1, exitCode);
		assertEquals(2, reporter.results.size(), "an INCOMPATIBLE result does not abort the run -- both reached");
		assertEquals(ValidationOutcome.INCOMPATIBLE, reporter.results.get(0).validationResult().outcome());
		assertEquals(null, reporter.results.get(0).applyResult(), "no Applier call for a non-GREEN outcome");
		assertEquals(ValidationOutcome.GREEN, reporter.results.get(1).validationResult().outcome());
		assertInstanceOf(CommittedNoPushAttempted.class, reporter.results.get(1).applyResult());
		assertEquals(1, git.commitCallCount, "only the GREEN candidate was committed");
	}
}
