package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.OptionalInt;

import org.junit.jupiter.api.Test;

/**
 * Pre-flight branch/JDK/dirty-tree logic (JDK-home resolution and
 * {@code git status} both faked -- no real subprocess), and per-target
 * {@code QUERY_FAILED} isolation not aborting the run.
 */
class DetectorTest {

	private static final Path FAKE_JDK_HOME = Path.of("fake-jdk-home");

	private DefaultDetector newDetector(Path pomPath, ToolConfiguration configuration, FakeJdkHomeResolver jdkHomes,
			FakeJdkVersionChecker jdkVersionChecker, FakeVersionRepositoryClient repo, FakeGitClient git) {
		return new DefaultDetector(pomPath, configuration, jdkHomes, jdkVersionChecker, repo, git);
	}

	@Test
	void validateEnvironment_matchingBranchAndJdkAndCleanTree_succeeds() throws Exception {
		FakeGitClient git = new FakeGitClient();
		git.branch = "master";
		git.statusPorcelainOutput = "";
		FakeJdkHomeResolver jdkHomes = new FakeJdkHomeResolver().with("master", FAKE_JDK_HOME);

		DefaultDetector detector = newDetector(Path.of("pom.xml"), ToolConfiguration.defaults(), jdkHomes,
				FakeJdkVersionChecker.returning(8), new FakeVersionRepositoryClient(), git);

		detector.validateEnvironment();

		assertEquals(FAKE_JDK_HOME, detector.getResolvedJdkHome());
	}

	@Test
	void validateEnvironment_jdkMajorVersionMismatch_throwsBranchJdkMismatch() {
		FakeGitClient git = new FakeGitClient();
		git.branch = "master"; // expects JDK 8
		FakeJdkHomeResolver jdkHomes = new FakeJdkHomeResolver().with("master", FAKE_JDK_HOME);

		DefaultDetector detector = newDetector(Path.of("pom.xml"), ToolConfiguration.defaults(), jdkHomes,
				FakeJdkVersionChecker.returning(25), new FakeVersionRepositoryClient(), git);

		EnvironmentCheckException ex = assertThrows(EnvironmentCheckException.class, detector::validateEnvironment);
		assertInstanceOf(BranchJdkMismatchException.class, ex);
	}

	@Test
	void validateEnvironment_noJdkHomeConfiguredForBranch_throwsBranchJdkMismatch() {
		FakeGitClient git = new FakeGitClient();
		git.branch = "v2.0-tc11";
		FakeJdkHomeResolver jdkHomes = new FakeJdkHomeResolver(); // nothing configured

		DefaultDetector detector = newDetector(Path.of("pom.xml"), ToolConfiguration.defaults(), jdkHomes,
				FakeJdkVersionChecker.returning(25), new FakeVersionRepositoryClient(), git);

		assertThrows(BranchJdkMismatchException.class, detector::validateEnvironment);
	}

	@Test
	void validateEnvironment_unrecognizedBranch_throwsBranchJdkMismatch() {
		FakeGitClient git = new FakeGitClient();
		git.branch = "some-feature-branch";
		FakeJdkHomeResolver jdkHomes = new FakeJdkHomeResolver().with("some-feature-branch", FAKE_JDK_HOME);

		DefaultDetector detector = newDetector(Path.of("pom.xml"), ToolConfiguration.defaults(), jdkHomes,
				FakeJdkVersionChecker.returning(8), new FakeVersionRepositoryClient(), git);

		assertThrows(BranchJdkMismatchException.class, detector::validateEnvironment);
	}

	@Test
	void validateEnvironment_configuredJdkHomeUnresolvable_throwsBranchJdkMismatch() {
		FakeGitClient git = new FakeGitClient();
		git.branch = "master";
		FakeJdkHomeResolver jdkHomes = new FakeJdkHomeResolver().with("master", FAKE_JDK_HOME);

		DefaultDetector detector = newDetector(Path.of("pom.xml"), ToolConfiguration.defaults(), jdkHomes,
				FakeJdkVersionChecker.unresolvable(), new FakeVersionRepositoryClient(), git);

		assertThrows(BranchJdkMismatchException.class, detector::validateEnvironment);
	}

	@Test
	void validateEnvironment_dirtyWorkingTree_throwsDirtyWorkingTree() {
		FakeGitClient git = new FakeGitClient();
		git.branch = "master";
		git.statusPorcelainOutput = " M pom.xml\n";
		FakeJdkHomeResolver jdkHomes = new FakeJdkHomeResolver().with("master", FAKE_JDK_HOME);

		DefaultDetector detector = newDetector(Path.of("pom.xml"), ToolConfiguration.defaults(), jdkHomes,
				FakeJdkVersionChecker.returning(8), new FakeVersionRepositoryClient(), git);

		EnvironmentCheckException ex = assertThrows(EnvironmentCheckException.class, detector::validateEnvironment);
		assertInstanceOf(DirtyWorkingTreeException.class, ex);
		assertTrue(ex.getMessage().contains("COMMIT_FAILED"),
				"message should hint at an unresolved prior COMMIT_FAILED (monitoring-design.md nuance)");
	}

	@Test
	void findCandidates_onePerTargetRepositoryQueryFailure_isIsolated_othersStillChecked() throws Exception {
		DependencyCoordinate good = new DependencyCoordinate("org.example", "good");
		DependencyCoordinate bad = new DependencyCoordinate("org.example", "bad");

		String pomXml = "<project><dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>good</artifactId><version>1.0.0</version></dependency>"
				+ "<dependency><groupId>org.example</groupId><artifactId>bad</artifactId><version>1.0.0</version></dependency>"
				+ "</dependencies></project>";
		Path pomPath = Files.createTempFile("pom", ".xml");
		Files.writeString(pomPath, pomXml);

		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient()
				.withVersions(good, "1.0.1")
				.withFailure(bad, "simulated network timeout");

		DefaultDetector detector = newDetector(pomPath, ToolConfiguration.defaults(), new FakeJdkHomeResolver(),
				FakeJdkVersionChecker.returning(8), repo, new FakeGitClient());

		List<DetectionResult> results = detector.findCandidates();

		assertEquals(2, results.size(), "both targets reported -- a failure does not abort the run (ADR-10)");
		assertInstanceOf(CandidateFound.class, results.get(0));
		assertInstanceOf(QueryFailed.class, results.get(1));
		assertEquals("simulated network timeout", ((QueryFailed) results.get(1)).diagnostic());

		Files.deleteIfExists(pomPath);
	}

	@Test
	void findCandidates_versionOutOfPolicy_isNoUpdateAvailable() throws Exception {
		DependencyCoordinate coord = new DependencyCoordinate("org.example", "widget");
		String pomXml = "<project><dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId><version>1.0.0</version></dependency>"
				+ "</dependencies></project>";
		Path pomPath = Files.createTempFile("pom", ".xml");
		Files.writeString(pomPath, pomXml);

		// Only a MAJOR bump is available; default policy is PATCH_ONLY.
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(coord, "2.0.0");

		DefaultDetector detector = newDetector(pomPath, ToolConfiguration.defaults(), new FakeJdkHomeResolver(),
				FakeJdkVersionChecker.returning(8), repo, new FakeGitClient());

		List<DetectionResult> results = detector.findCandidates();

		assertEquals(1, results.size());
		assertInstanceOf(NoUpdateAvailable.class, results.get(0));

		Files.deleteIfExists(pomPath);
	}

	@Test
	void findCandidates_testScopeDependency_isExcluded() throws Exception {
		String pomXml = "<project><dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId>"
				+ "<version>1.0.0</version><scope>test</scope></dependency>"
				+ "</dependencies></project>";
		Path pomPath = Files.createTempFile("pom", ".xml");
		Files.writeString(pomPath, pomXml);

		DefaultDetector detector = newDetector(pomPath, ToolConfiguration.defaults(), new FakeJdkHomeResolver(),
				FakeJdkVersionChecker.returning(8), new FakeVersionRepositoryClient(), new FakeGitClient());

		assertEquals(0, detector.findCandidates().size(), "FR-1.1: test-scope dependencies are out of scope");

		Files.deleteIfExists(pomPath);
	}

	@Test
	void findCandidates_selfNamespaceDependency_isExcluded() throws Exception {
		// BR-1b: a dependency sharing this project's own <groupId> (e.g. the
		// real org.tamacat:tamacat-core on master) is a sibling artifact, not a
		// third-party library, and is excluded regardless of what the
		// repository client would report for it.
		String pomXml = "<project><groupId>org.tamacat</groupId>"
				+ "<dependencies>"
				+ "<dependency><groupId>org.tamacat</groupId><artifactId>tamacat-core</artifactId>"
				+ "<version>1.5</version></dependency>"
				+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId>"
				+ "<version>1.0.0</version></dependency>"
				+ "</dependencies></project>";
		Path pomPath = Files.createTempFile("pom", ".xml");
		Files.writeString(pomPath, pomXml);

		DependencyCoordinate tamacatCore = new DependencyCoordinate("org.tamacat", "tamacat-core");
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient()
				.withFailure(tamacatCore, "should never be queried -- self-namespace is excluded before detection");

		DefaultDetector detector = newDetector(pomPath, ToolConfiguration.defaults(), new FakeJdkHomeResolver(),
				FakeJdkVersionChecker.returning(8), repo, new FakeGitClient());

		List<DetectionResult> results = detector.findCandidates();

		assertEquals(1, results.size(), "only the non-self-namespace dependency is a detection target");
		assertInstanceOf(NoUpdateAvailable.class, results.get(0));
		assertEquals(1, repo.queryCount, "tamacat-core must never reach the repository client at all");

		Files.deleteIfExists(pomPath);
	}

	@Test
	void jdkVersionProbe_parsesModernAndLegacyVersionStrings() {
		assertEquals(OptionalInt.of(25), ProcessJdkVersionChecker.parseMajorVersion(
				"openjdk version \"25.0.1\" 2025-10-21 LTS"));
		assertEquals(OptionalInt.of(8), ProcessJdkVersionChecker.parseMajorVersion(
				"java version \"1.8.0_462\""));
		assertEquals(OptionalInt.of(17), ProcessJdkVersionChecker.parseMajorVersion(
				"openjdk version \"17\" 2021-09-14"));
		assertEquals(OptionalInt.empty(), ProcessJdkVersionChecker.parseMajorVersion("not a version string"));
	}
}
