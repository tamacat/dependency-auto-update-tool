package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Set;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

/**
 * Candidate selection when the current version has known vulnerabilities
 * (securityFixPolicy): keep the normal candidate when it fixes them, escalate
 * up to the ceiling when it does not, and validate security fixes first.
 */
class SecurityFixDetectionTest {

	private static final DependencyCoordinate WIDGET = new DependencyCoordinate("org.example", "widget");
	private static final DependencyCoordinate OTHER = new DependencyCoordinate("org.example", "other");
	private static final Set<String> CVE = Set.of("GHSA-aaaa");

	private Path pomPath;

	@AfterEach
	void cleanUp() throws Exception {
		if (pomPath != null) {
			Files.deleteIfExists(pomPath);
		}
	}

	private List<DetectionResult> detect(String pomXml, ToolConfiguration config, FakeVersionRepositoryClient repo,
			FakeVulnerabilityClient vulns) throws Exception {
		pomPath = Files.createTempFile("pom", ".xml");
		Files.writeString(pomPath, pomXml);
		DefaultDetector detector = new DefaultDetector(pomPath, config, new FakeJdkHomeResolver(),
				FakeJdkVersionChecker.returning(8), repo, new FakeGitClient(), vulns);
		return detector.findCandidates();
	}

	private static String pom(String... artifactAndVersion) {
		StringBuilder sb = new StringBuilder("<project><dependencies>");
		for (int i = 0; i < artifactAndVersion.length; i += 2) {
			sb.append("<dependency><groupId>org.example</groupId><artifactId>").append(artifactAndVersion[i])
					.append("</artifactId><version>").append(artifactAndVersion[i + 1]).append("</version></dependency>");
		}
		return sb.append("</dependencies></project>").toString();
	}

	private static UpdateCandidate candidate(DetectionResult r) {
		return assertInstanceOf(CandidateFound.class, r).candidate();
	}

	@Test
	void notVulnerable_behavesAsBefore() throws Exception {
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET, "1.0.0", "1.0.1",
				"1.1.0");
		List<DetectionResult> results = detect(pom("widget", "1.0.0"), ToolConfiguration.defaults(), repo,
				new FakeVulnerabilityClient());

		UpdateCandidate c = candidate(results.get(0));
		assertEquals("1.0.1", c.candidateVersion(), "PATCH_ONLY still picks the newest patch");
		assertEquals(SecurityStatus.NONE, c.security());
	}

	@Test
	void normalCandidateFixesIt_isKeptAndMarkedAsSecurityFix() throws Exception {
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET, "1.0.0", "1.0.1",
				"1.0.2", "1.1.0");
		FakeVulnerabilityClient vulns = new FakeVulnerabilityClient().affected(WIDGET, CVE, "1.0.0");

		UpdateCandidate c = candidate(detect(pom("widget", "1.0.0"), ToolConfiguration.defaults(), repo, vulns).get(0));

		assertEquals("1.0.2", c.candidateVersion());
		assertEquals(CVE, c.security().fixed());
		assertFalse(c.security().escalated());
		assertTrue(CommitMessages.forCandidate(c).startsWith("fix(deps): bump org.example:widget from 1.0.0 to 1.0.2"));
		assertTrue(CommitMessages.forCandidate(c).contains("Fixes known vulnerabilities: GHSA-aaaa"));
	}

	@Test
	void patchDoesNotFixIt_escalatesToNewestFixingVersionOfSmallestFixingSeries() throws Exception {
		// 1.0.x is affected throughout; fixed in 1.1.2 (1.1.0-1.1.1 still affected); 1.2.x and 2.0.0 also fixed
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET, "1.0.0", "1.0.1",
				"1.1.0", "1.1.1", "1.1.2", "1.1.3", "1.2.0-RC1", "1.2.0", "2.0.0");
		FakeVulnerabilityClient vulns = new FakeVulnerabilityClient()
				.affected(WIDGET, CVE, "1.0.0", "1.0.1", "1.1.0", "1.1.1");
		List<DetectionResult> results = detect(pom("other", "3.0.0", "widget", "1.0.0"), ToolConfiguration.defaults(),
				repo.withVersions(OTHER, "3.0.0", "3.0.1"), vulns);

		UpdateCandidate c = candidate(results.get(0));
		assertEquals(WIDGET, ((DirectDependencyTarget) c.target()).coordinate(),
				"the security fix is validated first, ahead of 'other' that comes first in pom.xml");
		assertEquals("1.1.3", c.candidateVersion(), "newest fixing version in 1.1, not 1.2 or 2.0");
		assertTrue(c.security().escalated());
		assertTrue(CommitMessages.forCandidate(c).contains("larger bump than the configured version-range policy"));
		assertEquals("3.0.1", candidate(results.get(1)).candidateVersion());
	}

	@Test
	void onlyAMajorFixesIt_keepsNormalCandidateUnderMinorCeilingAndEscalatesUnderMajor() throws Exception {
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET, "1.0.0", "1.0.1",
				"1.1.0", "2.0.0");
		FakeVulnerabilityClient vulns = new FakeVulnerabilityClient()
				.affected(WIDGET, CVE, "1.0.0", "1.0.1", "1.1.0")
				.affected(WIDGET, Set.of("GHSA-bbbb"), "1.0.0");

		UpdateCandidate minor = candidate(detect(pom("widget", "1.0.0"), ToolConfiguration.defaults(), repo, vulns).get(0));
		assertEquals("1.0.1", minor.candidateVersion());
		assertEquals(Set.of("GHSA-bbbb"), minor.security().fixed());
		assertEquals(CVE, minor.security().remaining());
		assertTrue(CommitMessages.forCandidate(minor).contains("Still affected by: GHSA-aaaa"));

		UpdateCandidate major = candidate(detect(pom("widget", "1.0.0"),
				ToolConfiguration.defaults().withSecurityFixPolicy(SecurityFixPolicy.MAJOR), repo, vulns).get(0));
		assertEquals("2.0.0", major.candidateVersion());
		assertTrue(major.security().escalated());
	}

	@Test
	void vulnerableWithNoAllowedFix_isNoUpdateAvailableWithTheVulnerabilities() throws Exception {
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET, "1.0.0", "2.0.0");
		FakeVulnerabilityClient vulns = new FakeVulnerabilityClient().affected(WIDGET, CVE, "1.0.0");

		NoUpdateAvailable nu = assertInstanceOf(NoUpdateAvailable.class,
				detect(pom("widget", "1.0.0"), ToolConfiguration.defaults(), repo, vulns).get(0));
		assertEquals(CVE, nu.security().vulnerable());
		assertEquals(CVE, nu.security().remaining());
	}

	@Test
	void off_sendsNoQueries() throws Exception {
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET, "1.0.0", "1.0.1");
		FakeVulnerabilityClient vulns = new FakeVulnerabilityClient().affected(WIDGET, CVE, "1.0.0");

		UpdateCandidate c = candidate(detect(pom("widget", "1.0.0"),
				ToolConfiguration.defaults().withSecurityFixPolicy(SecurityFixPolicy.OFF), repo, vulns).get(0));
		assertEquals(SecurityStatus.NONE, c.security());
		assertTrue(vulns.queried.isEmpty());
	}

	@Test
	void lookupFailure_keepsNormalCandidateAndReportsWhy() throws Exception {
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient().withVersions(WIDGET, "1.0.0", "1.0.1");
		FakeVulnerabilityClient vulns = new FakeVulnerabilityClient();
		vulns.failure = "HTTP 503";

		UpdateCandidate c = candidate(detect(pom("widget", "1.0.0"), ToolConfiguration.defaults(), repo, vulns).get(0));
		assertEquals("1.0.1", c.candidateVersion());
		assertNotNull(c.security().diagnostic());
		assertTrue(c.security().diagnostic().contains("HTTP 503"));
	}

	@Test
	void propertyGroup_considersEveryMember() throws Exception {
		DependencyCoordinate core = new DependencyCoordinate("org.example", "core");
		DependencyCoordinate jasper = new DependencyCoordinate("org.example", "jasper");
		String pomXml = "<project><properties><lib.version>1.0.0</lib.version></properties><dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>core</artifactId><version>${lib.version}</version></dependency>"
				+ "<dependency><groupId>org.example</groupId><artifactId>jasper</artifactId><version>${lib.version}</version></dependency>"
				+ "</dependencies></project>";
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient()
				.withVersions(core, "1.0.0", "1.0.1", "1.1.0")
				.withVersions(jasper, "1.0.0", "1.0.1", "1.1.0");
		// Only jasper is affected, and still in 1.0.1
		FakeVulnerabilityClient vulns = new FakeVulnerabilityClient().affected(jasper, CVE, "1.0.0", "1.0.1");

		UpdateCandidate c = candidate(detect(pomXml, ToolConfiguration.defaults(), repo, vulns).get(0));
		assertEquals("1.1.0", c.candidateVersion());
		assertTrue(c.security().escalated());
	}
}
