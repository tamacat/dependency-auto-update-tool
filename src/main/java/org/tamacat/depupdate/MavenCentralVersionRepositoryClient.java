package org.tamacat.depupdate;

import java.io.IOException;
import java.io.StringReader;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;

import javax.xml.parsers.DocumentBuilder;
import javax.xml.parsers.DocumentBuilderFactory;

import org.w3c.dom.Document;
import org.w3c.dom.NodeList;
import org.xml.sax.InputSource;

/**
 * Queries a Maven repository's {@code maven-metadata.xml} directly over
 * HTTP, using the JDK's built-in {@link HttpClient} (tech-stack-decisions.md
 * -- no new HTTP dependency required, since this tool's own code targets a
 * modern Java version).
 */
public final class MavenCentralVersionRepositoryClient implements VersionRepositoryClient {

	private final HttpClient httpClient;
	private final String baseUrl;
	private final Duration timeout;

	public MavenCentralVersionRepositoryClient(String baseUrl, Duration timeout) {
		this.baseUrl = baseUrl.endsWith("/") ? baseUrl : baseUrl + "/";
		this.timeout = timeout;
		this.httpClient = HttpClient.newBuilder().connectTimeout(timeout).build();
	}

	@Override
	public List<String> queryAvailableVersions(DependencyCoordinate coordinate) throws RepositoryQueryException {
		String groupPath = coordinate.groupId().replace('.', '/');
		String url = baseUrl + groupPath + "/" + coordinate.artifactId() + "/maven-metadata.xml";
		try {
			HttpRequest request = HttpRequest.newBuilder(URI.create(url)).timeout(timeout).GET().build();
			HttpResponse<String> response = httpClient.send(request, HttpResponse.BodyHandlers.ofString());
			if (response.statusCode() != 200) {
				throw new RepositoryQueryException(
						"HTTP " + response.statusCode() + " querying " + url + " for " + coordinate);
			}
			return parseVersions(response.body());
		} catch (IOException e) {
			throw new RepositoryQueryException("Network error querying " + url + " for " + coordinate + ": "
					+ e.getMessage(), e);
		} catch (InterruptedException e) {
			Thread.currentThread().interrupt();
			throw new RepositoryQueryException("Interrupted while querying " + url + " for " + coordinate, e);
		}
	}

	static List<String> parseVersions(String metadataXml) throws RepositoryQueryException {
		try {
			DocumentBuilderFactory factory = DocumentBuilderFactory.newInstance();
			factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
			DocumentBuilder builder = factory.newDocumentBuilder();
			Document doc = builder.parse(new InputSource(new StringReader(metadataXml)));
			NodeList versionNodes = doc.getElementsByTagName("version");
			List<String> versions = new ArrayList<>();
			for (int i = 0; i < versionNodes.getLength(); i++) {
				String text = versionNodes.item(i).getTextContent();
				if (text != null && !text.isBlank()) {
					versions.add(text.trim());
				}
			}
			return versions;
		} catch (Exception e) {
			throw new RepositoryQueryException("Failed to parse maven-metadata.xml: " + e.getMessage(), e);
		}
	}
}
