package org.tamacat.depupdate;

import java.io.FilterInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Duration;
import java.util.OptionalInt;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

/**
 * Real {@link JavaRequirementClient}: downloads a version's jar from the same
 * Maven repository the versions are listed from (nothing is sent anywhere
 * else), reads the header of every class file in it, and takes the highest
 * class-file version.
 *
 * <p>Two kinds of entries are skipped, because they do not raise what the jar
 * needs: {@code module-info.class} (Java 8 never loads it, and libraries that
 * target Java 8 often compile it for Java 9), and anything under
 * {@code META-INF/versions/} (a multi-release jar's classes for newer Java
 * versions, which an older JVM ignores).
 */
public final class MavenJarJavaRequirementClient implements JavaRequirementClient {

	/** A jar larger than this is not read (no direct dependency should be anywhere near it). */
	static final long MAX_JAR_BYTES = 64L * 1024 * 1024;

	private static final int CLASS_MAGIC = 0xCAFEBABE;

	private final RetryingHttp http;
	private final String baseUrl;
	private final Duration timeout;

	public MavenJarJavaRequirementClient(String baseUrl, Duration timeout) {
		this(baseUrl, timeout, RetryingHttp.THREAD_SLEEP);
	}

	MavenJarJavaRequirementClient(String baseUrl, Duration timeout, RetryingHttp.Sleeper sleeper) {
		this.baseUrl = baseUrl.endsWith("/") ? baseUrl : baseUrl + "/";
		this.timeout = timeout;
		this.http = new RetryingHttp(HttpClient.newBuilder().connectTimeout(timeout)
				.followRedirects(HttpClient.Redirect.NORMAL).build(), sleeper);
	}

	@Override
	public OptionalInt requiredJavaMajor(DependencyCoordinate coordinate, String version)
			throws RepositoryQueryException {
		String url = baseUrl + coordinate.groupId().replace('.', '/') + "/" + coordinate.artifactId() + "/" + version
				+ "/" + coordinate.artifactId() + "-" + version + ".jar";
		try {
			HttpRequest request = HttpRequest.newBuilder(URI.create(url)).timeout(timeout).GET().build();
			HttpResponse<InputStream> response = http.send(request, HttpResponse.BodyHandlers.ofInputStream());
			try (InputStream body = response.body()) {
				if (response.statusCode() == 404) {
					// No jar (a pom-only artifact, for example): nothing to tell
					return OptionalInt.empty();
				}
				if (response.statusCode() != 200) {
					throw new RepositoryQueryException(
							"HTTP " + response.statusCode() + " downloading " + url + " for " + coordinate);
				}
				return requiredJavaMajor(new LimitedInputStream(body, MAX_JAR_BYTES));
			}
		} catch (IOException e) {
			throw new RepositoryQueryException(
					"Could not read " + url + " for " + coordinate + ": " + e.getMessage(), e);
		} catch (InterruptedException e) {
			Thread.currentThread().interrupt();
			throw new RepositoryQueryException("Interrupted while downloading " + url + " for " + coordinate, e);
		}
	}

	/** The Java version a jar needs, from its class files; empty when it has none. */
	static OptionalInt requiredJavaMajor(InputStream jar) throws IOException {
		int highest = -1;
		try (ZipInputStream zip = new ZipInputStream(jar)) {
			ZipEntry entry;
			while ((entry = zip.getNextEntry()) != null) {
				String name = entry.getName();
				if (entry.isDirectory() || !name.endsWith(".class") || name.startsWith("META-INF/versions/")
						|| name.equals("module-info.class") || name.endsWith("/module-info.class")) {
					continue;
				}
				byte[] header = zip.readNBytes(8);
				if (header.length < 8 || readInt(header, 0) != CLASS_MAGIC) {
					continue;
				}
				int classMajor = ((header[6] & 0xff) << 8) | (header[7] & 0xff);
				highest = Math.max(highest, classMajor);
			}
		}
		return highest < 0 ? OptionalInt.empty() : OptionalInt.of(javaMajor(highest));
	}

	/** Class-file major version to Java version: 52 is Java 8, 55 is Java 11, and so on. */
	static int javaMajor(int classFileMajor) {
		// 45-48 are Java 1.1-1.4; none of them is ever a target branch's JDK
		return Math.max(1, classFileMajor - 44);
	}

	private static int readInt(byte[] b, int offset) {
		return ((b[offset] & 0xff) << 24) | ((b[offset + 1] & 0xff) << 16) | ((b[offset + 2] & 0xff) << 8)
				| (b[offset + 3] & 0xff);
	}

	/** Fails instead of reading past a size limit. */
	private static final class LimitedInputStream extends FilterInputStream {

		private final long limit;
		private long count;

		LimitedInputStream(InputStream in, long limit) {
			super(in);
			this.limit = limit;
		}

		@Override
		public int read() throws IOException {
			int b = super.read();
			if (b >= 0) {
				count(1);
			}
			return b;
		}

		@Override
		public int read(byte[] buf, int off, int len) throws IOException {
			int n = super.read(buf, off, len);
			if (n > 0) {
				count(n);
			}
			return n;
		}

		private void count(long n) throws IOException {
			count += n;
			if (count > limit) {
				throw new IOException("jar is larger than " + limit + " bytes");
			}
		}
	}
}
