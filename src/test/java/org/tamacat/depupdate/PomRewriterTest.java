package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;

import org.junit.jupiter.api.Test;

/**
 * business-rules.md BR-4 step 1, plus code-generation review Findings 4/5:
 * XML-escaping the injected version text, and never mistaking a
 * {@code <dependencyManagement>}-nested block for the top-level declaration.
 */
class PomRewriterTest {

	@Test
	void rewriteDependencyVersion_directTarget_touchesOnlyThatElement() {
		String pomXml = "<project><dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId><version>1.0.0</version></dependency>"
				+ "</dependencies></project>";
		DependencyCoordinate coordinate = new DependencyCoordinate("org.example", "widget");

		String result = PomRewriter.rewriteVersion(pomXml, new DirectDependencyTarget(coordinate, "1.0.0"), "1.1.0");

		assertEquals("<project><dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId><version>1.1.0</version></dependency>"
				+ "</dependencies></project>", result);
	}

	@Test
	void rewriteProperty_propertyIndirectedTarget_touchesOnlyThatProperty() {
		String pomXml = "<project><properties><tomcat.version>9.0.0</tomcat.version></properties>"
				+ "<dependencies>"
				+ "<dependency><groupId>org.apache.tomcat.embed</groupId><artifactId>tomcat-embed-core</artifactId>"
				+ "<version>${tomcat.version}</version></dependency>"
				+ "</dependencies></project>";
		PropertyIndirectedTarget target = new PropertyIndirectedTarget("tomcat.version",
				List.of(new DependencyCoordinate("org.apache.tomcat.embed", "tomcat-embed-core")), "9.0.0");

		String result = PomRewriter.rewriteVersion(pomXml, target, "9.0.1");

		assertTrue(result.contains("<tomcat.version>9.0.1</tomcat.version>"));
		assertTrue(result.contains("<version>${tomcat.version}</version>"),
				"the dependency's own ${property} reference must be untouched");
	}

	@Test
	void rewriteDependencyVersion_sameCoordinateInDependencyManagement_rewritesOnlyTheTopLevelDeclaration() {
		// Finding 5: a <dependencyManagement>-nested block sharing the same
		// coordinate as the real, top-level dependency must never be the one
		// rewritten -- PomParser only ever reports the top-level declaration
		// as the detection target.
		String pomXml = "<project>"
				+ "<dependencyManagement><dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId><version>0.9.0</version></dependency>"
				+ "</dependencies></dependencyManagement>"
				+ "<dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId><version>1.0.0</version></dependency>"
				+ "</dependencies></project>";
		DependencyCoordinate coordinate = new DependencyCoordinate("org.example", "widget");

		String result = PomRewriter.rewriteVersion(pomXml, new DirectDependencyTarget(coordinate, "1.0.0"), "1.1.0");

		assertTrue(result.contains("<dependencyManagement><dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId><version>0.9.0</version></dependency>"),
				"the dependencyManagement-nested declaration must be left exactly as-is");
		assertFalse(result.contains("<version>1.1.0</version></dependency></dependencies></dependencyManagement>"),
				"the new version must never land inside the dependencyManagement block");
		assertTrue(result.endsWith("<dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId><version>1.1.0</version></dependency>"
				+ "</dependencies></project>"),
				"the top-level declaration is the one that gets rewritten");
	}

	@Test
	void rewriteDependencyVersion_versionContainingXmlMetacharacters_isEscaped() {
		// Finding 4: a repository-supplied version string is untrusted input at
		// this system boundary -- splicing it in unescaped could corrupt the
		// surrounding XML.
		String pomXml = "<project><dependencies>"
				+ "<dependency><groupId>org.example</groupId><artifactId>widget</artifactId><version>1.0.0</version></dependency>"
				+ "</dependencies></project>";
		DependencyCoordinate coordinate = new DependencyCoordinate("org.example", "widget");

		String result = PomRewriter.rewriteVersion(pomXml, new DirectDependencyTarget(coordinate, "1.0.0"),
				"1.0.0<injected>&");

		assertTrue(result.contains("<version>1.0.0&lt;injected&gt;&amp;</version>"));
	}

	@Test
	void rewriteProperty_versionContainingXmlMetacharacters_isEscaped() {
		// Finding 4 (§12a review iteration 2 residual coverage gap): the
		// PropertyIndirectedTarget path shares the same escapeXmlText() call as
		// the DirectDependencyTarget path -- covered separately here.
		String pomXml = "<project><properties><tomcat.version>9.0.0</tomcat.version></properties>"
				+ "<dependencies>"
				+ "<dependency><groupId>org.apache.tomcat.embed</groupId><artifactId>tomcat-embed-core</artifactId>"
				+ "<version>${tomcat.version}</version></dependency>"
				+ "</dependencies></project>";
		PropertyIndirectedTarget target = new PropertyIndirectedTarget("tomcat.version",
				List.of(new DependencyCoordinate("org.apache.tomcat.embed", "tomcat-embed-core")), "9.0.0");

		String result = PomRewriter.rewriteVersion(pomXml, target, "9.0.0<injected>&");

		assertTrue(result.contains("<tomcat.version>9.0.0&lt;injected&gt;&amp;</tomcat.version>"));
	}
}
