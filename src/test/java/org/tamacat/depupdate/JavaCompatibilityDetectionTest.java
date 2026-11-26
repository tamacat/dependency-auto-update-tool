package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

/**
 * Newer versions whose jars need a newer Java than the branch's JDK are left
 * out before a candidate is chosen, for security fixes too.
 */
class JavaCompatibilityDetectionTest {

	private static final DependencyCoordinate WIDGET = new DependencyCoordinate("org.example", "widget");
	private static final Set<String> CVE = Set.of("GHSA-aaaa");

	private Path pomPath;

	@AfterEach
	void cleanUp() throws Exception {
		if (pomPath != null) {
			Files.deleteIfExists(pomPath);
		}
	}

	/** Runs pre-flight for a branch whose JDK is {@code jdk}, then detection. */
	private List<DetectionResult> detect(String pomXml, int jdk, FakeVersionRepositoryClient repo,
			FakeVulnerabilityClient vulns, JavaRequirementClient java) throws Exception {
		pomPath = Files.createTempFile("pom", ".xml");
		Files.writeString(pomPath, pomXml);
		FakeGitClient git = new FakeGitClient();
		git.branch = "v1.6";
		DefaultDetector detector = new DefaultDetector(pomPath,
				ToolConfiguration.defaults().withExpectedJdkMajor("v1.6", jdk),
				new FakeJdkHomeResolver().with("v1.6", Path.of("fake-jdk-home")), FakeJdkVersionChecker.returning(jdk),
				repo, git, vulns, java);
		detector.validateEnvironment();
		return detector.findCandidates();
	}

	private static String pom(String artifactId, String version) {
		return "<project><dependencies><dependency><groupId>org.example</groupId><artifactId>" + artifactId
				+ "</artifactId><version>" + version + "</version></dependency></dependencies></project>";
	}

	/** tamacat-httpd v1.6: logback-core 1.3.16 is vulnerable and only 1.5.x (Java 11) fixes it. */
	private static FakeVersionRepositoryClient logbackLikeVersions() {
		return new FakeVersionRepositoryClient().withVersions(WIDGET, "1.3.15", "1.3.16", "1.4.0", "1.4.14", "1.5.25",
				"1.5.38");
	}

	private static FakeVulnerabilityClient logbackLikeVulnerabilities() {
		return new FakeVulnerabilityClient().affected(WIDGET, CVE, "1.3.15", "1.3.16", "1.4.0", "1.4.14");
	}

	@Test
	void securityFixNeedingNewerJava_isLeftOut_andReportedAsUnfixed() throws Exception {
		FakeJavaRequirementClient java = new FakeJavaRequirementClient().needs(WIDGET, "1.4.0", 11);

		DetectionResult r = detect(pom("widget", "1.3.16"), 8, logbackLikeVersions(), logbackLikeVulnerabilities(),
				java).get(0);

		NoUpdateAvailable nu = assertInstanceOf(NoUpdateAvailable.class, r);
		assertEquals(CVE, nu.security().vulnerable());
		assertEquals(CVE, nu.security().remaining(), "nothing that runs on Java 8 fixes it");
		assertEquals(new JavaExclusion("1.4.0", 11, 8), nu.javaExclusion());
	}

	@Test
	void withoutTheCheck_theJava11FixIsProposed() throws Exception {
		// What happened before the check existed
		DetectionResult r = detect(pom("widget", "1.3.16"), 8, logbackLikeVersions(), logbackLikeVulnerabilities(),
				null).get(0);

		assertEquals("1.5.38", assertInstanceOf(CandidateFound.class, r).candidate().candidateVersion());
	}

	@Test
	void securityFixNeedingNewerJava_isKept_whenTheBranchJdkIsNewEnough() throws Exception {
		FakeJavaRequirementClient java = new FakeJavaRequirementClient().needs(WIDGET, "1.4.0", 11);

		DetectionResult r = detect(pom("widget", "1.3.16"), 11, logbackLikeVersions(), logbackLikeVulnerabilities(),
				java).get(0);

		UpdateCandidate c = assertInstanceOf(CandidateFound.class, r).candidate();
		assertEquals("1.5.38", c.candidateVersion());
		assertNull(c.javaExclusion());
	}

	@Test
	void newestPatchNeedingNewerJava_fallsBackToTheNewestThatRuns() throws Exception {
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET, "1.0.0", "1.0.1",
				"1.0.2", "1.0.3");
		FakeJavaRequirementClient java = new FakeJavaRequirementClient().needs(WIDGET, "1.0.3", 17);

		DetectionResult r = detect(pom("widget", "1.0.0"), 8, repo, new FakeVulnerabilityClient(), java).get(0);

		UpdateCandidate c = assertInstanceOf(CandidateFound.class, r).candidate();
		assertEquals("1.0.2", c.candidateVersion());
		assertEquals(new JavaExclusion("1.0.3", 17, 8), c.javaExclusion());
	}

	@Test
	void newestVersionRuns_onlyItsJarIsRead() throws Exception {
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET, "1.0.0", "1.0.1",
				"1.0.2", "1.1.0");
		FakeJavaRequirementClient java = new FakeJavaRequirementClient();

		UpdateCandidate c = assertInstanceOf(CandidateFound.class,
				detect(pom("widget", "1.0.0"), 8, repo, new FakeVulnerabilityClient(), java).get(0)).candidate();

		assertEquals("1.0.2", c.candidateVersion());
		assertNull(c.javaExclusion());
		assertEquals(List.of("widget:1.1.0"), java.queried);
	}

	@Test
	void manyVersions_theCutOffIsFoundByBinarySearch() throws Exception {
		List<String> versions = new ArrayList<>();
		for (int i = 0; i <= 64; i++) {
			versions.add("1.0." + i);
		}
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET,
				versions.toArray(String[]::new));
		FakeJavaRequirementClient java = new FakeJavaRequirementClient().needs(WIDGET, "1.0.41", 11);

		UpdateCandidate c = assertInstanceOf(CandidateFound.class,
				detect(pom("widget", "1.0.0"), 8, repo, new FakeVulnerabilityClient(), java).get(0)).candidate();

		assertEquals("1.0.40", c.candidateVersion());
		assertEquals("1.0.41", c.javaExclusion().fromVersion());
		assertTrue(java.queried.size() <= 8, "read " + java.queried.size() + " jars: " + java.queried);
	}

	@Test
	void propertyGroup_aMemberNeedingNewerJava_leavesTheVersionOutForAll() throws Exception {
		DependencyCoordinate core = new DependencyCoordinate("org.example", "core");
		DependencyCoordinate jasper = new DependencyCoordinate("org.example", "jasper");
		String pomXml = "<project><properties><lib.version>1.0.0</lib.version></properties><dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>core</artifactId><version>${lib.version}</version></dependency>"
				+ "<dependency><groupId>org.example</groupId><artifactId>jasper</artifactId><version>${lib.version}</version></dependency>"
				+ "</dependencies></project>";
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient()
				.withVersions(core, "1.0.0", "1.0.1", "1.0.2")
				.withVersions(jasper, "1.0.0", "1.0.1", "1.0.2");
		FakeJavaRequirementClient java = new FakeJavaRequirementClient().needs(jasper, "1.0.2", 11);

		UpdateCandidate c = assertInstanceOf(CandidateFound.class,
				detect(pomXml, 8, repo, new FakeVulnerabilityClient(), java).get(0)).candidate();

		assertEquals("1.0.1", c.candidateVersion());
		assertEquals(new JavaExclusion("1.0.2", 11, 8), c.javaExclusion());
	}

	@Test
	void jarCannotBeRead_isQueryFailed() throws Exception {
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET, "1.0.0", "1.0.1");
		FakeJavaRequirementClient java = new FakeJavaRequirementClient().failing(WIDGET, "HTTP 503 downloading jar");

		QueryFailed qf = assertInstanceOf(QueryFailed.class,
				detect(pom("widget", "1.0.0"), 8, repo, new FakeVulnerabilityClient(), java).get(0));

		assertTrue(qf.diagnostic().contains("HTTP 503"));
	}

	@Test
	void reportLine_saysWhichVersionsWereLeftOut() {
		RecordingConsole console = new RecordingConsole();
		DefaultReporter reporter = new DefaultReporter(console.stream(), Path.of("target", "test-reports"));
		reporter.recordDetection(new NoUpdateAvailable(new DirectDependencyTarget(WIDGET, "1.3.16"),
				new SecurityStatus(CVE, Set.of(), CVE, false, null), new JavaExclusion("1.4.0", 11, 8)));

		assertTrue(console.text().contains("outcome=NO_UPDATE_AVAILABLE vulnerable=\"GHSA-aaaa\" fixes=\"\" "
				+ "remaining=\"GHSA-aaaa\" javaExcluded=\"1.4.0 and newer need Java 11 (branch JDK 8)\""),
				console.text());
	}

	/** Captures what DefaultReporter prints. */
	private static final class RecordingConsole {

		private final java.io.ByteArrayOutputStream bytes = new java.io.ByteArrayOutputStream();

		java.io.PrintStream stream() {
			return new java.io.PrintStream(bytes, true, java.nio.charset.StandardCharsets.UTF_8);
		}

		String text() {
			return bytes.toString(java.nio.charset.StandardCharsets.UTF_8);
		}
	}
}
