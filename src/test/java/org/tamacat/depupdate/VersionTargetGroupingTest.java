package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import java.util.Map;

import org.junit.jupiter.api.Test;

/**
 * business-rules.md BR-1a: {@link VersionTarget} grouping, effective policy,
 * and (indirectly, via {@link DefaultDetector}) candidate-version selection.
 */
class VersionTargetGroupingTest {

	private static final String REAL_TOMCAT_POM_FRAGMENT_PROPERTIES = "tomcat.version";

	@Test
	void realPomShape_twoDependenciesSharingTomcatVersionProperty_groupIntoOnePropertyIndirectedTarget() {
		// The exact real-world shape from this repository's own pom.xml:
		// tomcat-embed-core and tomcat-embed-jasper both declare
		// <version>${tomcat.version}</version>.
		DependencyCoordinate core = new DependencyCoordinate("org.apache.tomcat.embed", "tomcat-embed-core");
		DependencyCoordinate jasper = new DependencyCoordinate("org.apache.tomcat.embed", "tomcat-embed-jasper");
		DependencyCoordinate httpcore5 = new DependencyCoordinate("org.apache.httpcomponents.core5", "httpcore5");

		List<RawDependencyDeclaration> declarations = List.of(
				new RawDependencyDeclaration(httpcore5, "5.5-beta2"),
				new RawDependencyDeclaration(core, "${" + REAL_TOMCAT_POM_FRAGMENT_PROPERTIES + "}"),
				new RawDependencyDeclaration(jasper, "${" + REAL_TOMCAT_POM_FRAGMENT_PROPERTIES + "}"));
		Map<String, String> properties = Map.of(REAL_TOMCAT_POM_FRAGMENT_PROPERTIES, "11.0.25");

		List<VersionTarget> targets = VersionTargetGrouper.group(declarations, properties);

		assertEquals(2, targets.size(), "httpcore5 (direct) + one grouped tomcat.version target");

		VersionTarget first = targets.get(0);
		assertInstanceOf(DirectDependencyTarget.class, first);
		DirectDependencyTarget direct = (DirectDependencyTarget) first;
		assertEquals(httpcore5, direct.coordinate());
		assertEquals("5.5-beta2", direct.currentVersion());

		VersionTarget second = targets.get(1);
		assertInstanceOf(PropertyIndirectedTarget.class, second);
		PropertyIndirectedTarget grouped = (PropertyIndirectedTarget) second;
		assertEquals(REAL_TOMCAT_POM_FRAGMENT_PROPERTIES, grouped.propertyName());
		assertEquals("11.0.25", grouped.currentVersion());
		assertEquals(List.of(core, jasper), grouped.members(), "members in first-encounter order, both present");
	}

	@Test
	void dependencyWithLiteralVersion_isNeverGrouped() {
		DependencyCoordinate a = new DependencyCoordinate("org.example", "a");
		List<RawDependencyDeclaration> declarations = List.of(new RawDependencyDeclaration(a, "1.0.0"));

		List<VersionTarget> targets = VersionTargetGrouper.group(declarations, Map.of());

		assertEquals(1, targets.size());
		assertInstanceOf(DirectDependencyTarget.class, targets.get(0));
	}

	@Test
	void propertyReferencedButNotDeclared_throws() {
		DependencyCoordinate a = new DependencyCoordinate("org.example", "a");
		List<RawDependencyDeclaration> declarations = List.of(new RawDependencyDeclaration(a, "${missing.prop}"));

		assertThrows(IllegalStateException.class, () -> VersionTargetGrouper.group(declarations, Map.of()));
	}

	@Test
	void effectivePolicy_directTarget_usesConfiguredPolicyOrDefaultsToPatchOnly() {
		DependencyCoordinate a = new DependencyCoordinate("org.example", "a");
		DirectDependencyTarget target = (DirectDependencyTarget) VersionTargetGrouper
				.group(List.of(new RawDependencyDeclaration(a, "1.0.0")), Map.of()).get(0);

		assertEquals(VersionRangePolicy.PATCH_ONLY,
				VersionTargetGrouper.effectivePolicy(target, Map.of()), "no config entry -> default PATCH_ONLY");
		assertEquals(VersionRangePolicy.MAJOR,
				VersionTargetGrouper.effectivePolicy(target, Map.of(a, VersionRangePolicy.MAJOR)));
	}

	@Test
	void effectivePolicy_propertyIndirectedTarget_isMostRestrictiveAmongMembers() {
		DependencyCoordinate core = new DependencyCoordinate("org.apache.tomcat.embed", "tomcat-embed-core");
		DependencyCoordinate jasper = new DependencyCoordinate("org.apache.tomcat.embed", "tomcat-embed-jasper");
		List<VersionTarget> targets = VersionTargetGrouper.group(
				List.of(new RawDependencyDeclaration(core, "${tomcat.version}"),
						new RawDependencyDeclaration(jasper, "${tomcat.version}")),
				Map.of("tomcat.version", "11.0.25"));
		PropertyIndirectedTarget target = (PropertyIndirectedTarget) targets.get(0);

		// core configured MAJOR, jasper left at default PATCH_ONLY -> most restrictive wins.
		VersionRangePolicy effective = VersionTargetGrouper.effectivePolicy(target,
				Map.of(core, VersionRangePolicy.MAJOR));
		assertEquals(VersionRangePolicy.PATCH_ONLY, effective);
	}

	@Test
	void candidateVersionSelection_realTomcatFixture_endToEndThroughDetector_intersectsAndPicksHighest() throws Exception {
		// End-to-end through DefaultDetector.findCandidates(), exercising the real
		// grouping + intersection + BR-1/BR-2 classification against the exact
		// two-member property-indirected shape found in this repository's pom.xml.
		DependencyCoordinate core = new DependencyCoordinate("org.apache.tomcat.embed", "tomcat-embed-core");
		DependencyCoordinate jasper = new DependencyCoordinate("org.apache.tomcat.embed", "tomcat-embed-jasper");

		String pomXml = "<project>"
				+ "<properties><tomcat.version>11.0.25</tomcat.version></properties>"
				+ "<dependencies>"
				+ "<dependency><groupId>org.apache.tomcat.embed</groupId><artifactId>tomcat-embed-core</artifactId>"
				+ "<version>${tomcat.version}</version></dependency>"
				+ "<dependency><groupId>org.apache.tomcat.embed</groupId><artifactId>tomcat-embed-jasper</artifactId>"
				+ "<version>${tomcat.version}</version></dependency>"
				+ "</dependencies></project>";
		java.nio.file.Path pomPath = java.nio.file.Files.createTempFile("pom", ".xml");
		java.nio.file.Files.writeString(pomPath, pomXml);

		// core has 11.0.26 and 11.0.27 available; jasper only has 11.0.26 -- the
		// intersection must exclude 11.0.27 (BR-1a step 3: "never a candidate that
		// only some members actually have available").
		FakeVersionRepositoryClient repo = new FakeVersionRepositoryClient()
				.withVersions(core, "11.0.25", "11.0.26", "11.0.27")
				.withVersions(jasper, "11.0.25", "11.0.26");

		DefaultDetector detector = new DefaultDetector(pomPath, ToolConfiguration.defaults(),
				new FakeJdkHomeResolver(), FakeJdkVersionChecker.returning(25), repo, new FakeGitClient());

		List<DetectionResult> results = detector.findCandidates();

		assertEquals(1, results.size());
		assertInstanceOf(CandidateFound.class, results.get(0));
		UpdateCandidate candidate = ((CandidateFound) results.get(0)).candidate();
		assertEquals("11.0.25", candidate.currentVersion());
		assertEquals("11.0.26", candidate.candidateVersion(), "11.0.27 excluded: not available for jasper");
		assertTrue(candidate.target() instanceof PropertyIndirectedTarget);

		java.nio.file.Files.deleteIfExists(pomPath);
	}
}
