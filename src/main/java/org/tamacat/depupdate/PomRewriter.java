package org.tamacat.depupdate;

import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * business-rules.md BR-4 step 1: temporarily rewrites {@code pom.xml} --
 * for a {@code DirectDependencyTarget}, the matching {@code <dependency>}'s
 * {@code <version>} element becomes the candidate version; for a
 * {@code PropertyIndirectedTarget}, the {@code <properties>} entry named
 * {@code target.propertyName} becomes the candidate version -- the
 * {@code <dependency>} elements referencing {@code ${propertyName}} are
 * never touched, preserving the indirection.
 *
 * <p>Deliberately text-based (not DOM-serialize-and-write-back): a DOM
 * round-trip would reformat the whole file, producing a much larger diff
 * than the single-element/single-property change BR-4 calls for. Each
 * rewrite touches exactly one text node's content and leaves every other
 * byte of the file untouched.
 */
public final class PomRewriter {

	private PomRewriter() {
	}

	public static String rewriteVersion(String pomXml, VersionTarget target, String newVersion) {
		if (target instanceof DirectDependencyTarget ddt) {
			return rewriteDependencyVersion(pomXml, ddt.coordinate(), newVersion);
		}
		if (target instanceof PropertyIndirectedTarget pit) {
			return rewriteProperty(pomXml, pit.propertyName(), newVersion);
		}
		throw new IllegalArgumentException("Unknown VersionTarget implementation: " + target.getClass());
	}

	static String rewriteDependencyVersion(String pomXml, DependencyCoordinate coordinate, String newVersion) {
		Pattern blockPattern = Pattern.compile("<dependency>.*?</dependency>", Pattern.DOTALL);
		Matcher blockMatcher = blockPattern.matcher(pomXml);
		while (blockMatcher.find()) {
			if (isInsideDependencyManagement(pomXml, blockMatcher.start())) {
				// Mirrors PomParser's own direct-children-only scoping (Finding 5,
				// code-generation review): a <dependencyManagement>-nested block
				// sharing the same coordinate must never be mistaken for the
				// top-level declaration PomParser reported as the detection target.
				continue;
			}
			String block = blockMatcher.group();
			if (containsElementWithValue(block, "groupId", coordinate.groupId())
					&& containsElementWithValue(block, "artifactId", coordinate.artifactId())) {
				String rewrittenBlock = replaceVersionElement(block, newVersion);
				return pomXml.substring(0, blockMatcher.start()) + rewrittenBlock
						+ pomXml.substring(blockMatcher.end());
			}
		}
		throw new IllegalStateException(
				"Could not locate a <dependency> block for " + coordinate + " to rewrite its version");
	}

	private static boolean isInsideDependencyManagement(String pomXml, int position) {
		int lastOpen = pomXml.lastIndexOf("<dependencyManagement>", position);
		if (lastOpen == -1) {
			return false;
		}
		int lastClose = pomXml.lastIndexOf("</dependencyManagement>", position);
		return lastOpen > lastClose;
	}

	static String rewriteProperty(String pomXml, String propertyName, String newVersion) {
		String quotedName = Pattern.quote(propertyName);
		Pattern propertyPattern = Pattern.compile("(<" + quotedName + ">)\\s*[^<]*?\\s*(</" + quotedName + ">)");
		Matcher m = propertyPattern.matcher(pomXml);
		if (!m.find()) {
			throw new IllegalStateException(
					"Could not locate <properties>/<" + propertyName + "> to rewrite its version");
		}
		String replacement = m.group(1) + escapeXmlText(newVersion) + m.group(2);
		return pomXml.substring(0, m.start()) + replacement + pomXml.substring(m.end());
	}

	private static String replaceVersionElement(String block, String newVersion) {
		Pattern versionPattern = Pattern.compile("(<version>)\\s*[^<]*?\\s*(</version>)");
		Matcher m = versionPattern.matcher(block);
		if (!m.find()) {
			throw new IllegalStateException(
					"<dependency> block has no <version> element to rewrite (a property-indirected "
							+ "dependency should never reach this method)");
		}
		String replacement = m.group(1) + escapeXmlText(newVersion) + m.group(2);
		return block.substring(0, m.start()) + replacement + block.substring(m.end());
	}

	private static boolean containsElementWithValue(String block, String tagName, String value) {
		Pattern p = Pattern.compile("<" + tagName + ">\\s*" + Pattern.quote(value) + "\\s*</" + tagName + ">");
		return p.matcher(block).find();
	}

	/**
	 * Escapes the characters that are significant in XML text content
	 * (Finding 4, code-generation review): a repository-supplied version
	 * string is untrusted input at this system boundary, and splicing it in
	 * unescaped could corrupt the surrounding XML. In practice a corrupted
	 * document fails {@code mvn clean verify} outright (never {@code GREEN}),
	 * but escaping here is the correct boundary behavior regardless.
	 */
	private static String escapeXmlText(String text) {
		return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;");
	}
}
