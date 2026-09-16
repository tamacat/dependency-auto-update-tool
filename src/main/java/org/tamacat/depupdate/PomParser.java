package org.tamacat.depupdate;

import java.io.StringReader;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import javax.xml.parsers.DocumentBuilder;
import javax.xml.parsers.DocumentBuilderFactory;

import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.Node;
import org.w3c.dom.NodeList;
import org.xml.sax.InputSource;

/**
 * Reads {@code pom.xml} for detection: the {@code <properties>} map and the
 * direct, non-test-scope, non-self-namespace {@code <dependency>}
 * declarations (FR-1.1, BR-1b).
 *
 * <p>Only the root {@code <project>}'s direct child {@code <dependencies>}
 * element's direct child {@code <dependency>} elements are considered --
 * deliberately not a deep {@code getElementsByTagName} search, so a
 * {@code <dependencyManagement>} block (this repository's {@code pom.xml}
 * has none today, but a future one might) is never mistaken for a direct
 * dependency declaration.
 */
public final class PomParser {

	private PomParser() {
	}

	public static Map<String, String> parseProperties(String pomXml) {
		Document doc = parseDocument(pomXml);
		Map<String, String> properties = new LinkedHashMap<>();
		Element propertiesEl = firstDirectChild(doc.getDocumentElement(), "properties");
		if (propertiesEl == null) {
			return properties;
		}
		NodeList children = propertiesEl.getChildNodes();
		for (int i = 0; i < children.getLength(); i++) {
			Node n = children.item(i);
			if (n.getNodeType() == Node.ELEMENT_NODE) {
				properties.put(n.getNodeName(), n.getTextContent().trim());
			}
		}
		return properties;
	}

	public static List<RawDependencyDeclaration> parseDependencies(String pomXml) {
		Document doc = parseDocument(pomXml);
		// BR-1b: a dependency sharing this project's own <groupId> is a sibling
		// artifact of the same project (e.g. org.tamacat:tamacat-core), not a
		// third-party OSS library -- it is excluded from detection entirely.
		// Such artifacts are frequently published to a project-specific
		// repository rather than Maven Central, which this tool's single
		// configured repository client cannot be expected to resolve (Change
		// Request, code-generation review iteration 1, 2026-09-05).
		String projectGroupId = textOf(doc.getDocumentElement(), "groupId");
		Element dependenciesEl = firstDirectChild(doc.getDocumentElement(), "dependencies");
		List<RawDependencyDeclaration> result = new ArrayList<>();
		if (dependenciesEl == null) {
			return result;
		}
		NodeList children = dependenciesEl.getChildNodes();
		for (int i = 0; i < children.getLength(); i++) {
			Node n = children.item(i);
			if (n.getNodeType() != Node.ELEMENT_NODE || !"dependency".equals(n.getNodeName())) {
				continue;
			}
			Element depEl = (Element) n;
			String groupId = textOf(depEl, "groupId");
			String artifactId = textOf(depEl, "artifactId");
			String scope = textOf(depEl, "scope");
			String version = textOf(depEl, "version");
			if (groupId == null || artifactId == null) {
				continue;
			}
			if ("test".equals(scope)) {
				// FR-1.1: test-scope dependencies are explicitly out of scope.
				continue;
			}
			if (groupId.equals(projectGroupId)) {
				// BR-1b: self-namespace dependency -- see comment above.
				continue;
			}
			if (version == null) {
				// No <version> to form a VersionTarget from (e.g. inherited from
				// dependencyManagement) -- not representable by this tool's model;
				// skip rather than crash on an unexpected pom.xml shape.
				continue;
			}
			result.add(new RawDependencyDeclaration(new DependencyCoordinate(groupId, artifactId), version));
		}
		return result;
	}

	private static Element firstDirectChild(Element parent, String tagName) {
		NodeList children = parent.getChildNodes();
		for (int i = 0; i < children.getLength(); i++) {
			Node n = children.item(i);
			if (n.getNodeType() == Node.ELEMENT_NODE && tagName.equals(n.getNodeName())) {
				return (Element) n;
			}
		}
		return null;
	}

	private static String textOf(Element parent, String tagName) {
		Element child = firstDirectChild(parent, tagName);
		return child == null ? null : child.getTextContent().trim();
	}

	private static Document parseDocument(String pomXml) {
		try {
			DocumentBuilderFactory factory = DocumentBuilderFactory.newInstance();
			// Defense-in-depth against XXE (Construction phase guardrail: validate/
			// sanitize inputs at system boundaries) -- pom.xml is a trusted local
			// file in practice, but there is no reason to allow DOCTYPE processing.
			factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
			factory.setXIncludeAware(false);
			factory.setExpandEntityReferences(false);
			DocumentBuilder builder = factory.newDocumentBuilder();
			return builder.parse(new InputSource(new StringReader(pomXml)));
		} catch (Exception e) {
			throw new IllegalStateException("Failed to parse pom.xml: " + e.getMessage(), e);
		}
	}
}

/** One {@code <dependency>} declaration's raw, unresolved version text (literal, or {@code ${property}}). */
record RawDependencyDeclaration(DependencyCoordinate coordinate, String versionText) {
}
