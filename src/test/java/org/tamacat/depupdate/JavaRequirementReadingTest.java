package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.net.InetSocketAddress;
import java.time.Duration;
import java.util.OptionalInt;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

import org.junit.jupiter.api.Test;

import com.sun.net.httpserver.HttpServer;

/** Reading the Java a jar needs from its class-file headers. */
class JavaRequirementReadingTest {

	private static final DependencyCoordinate LOGBACK = new DependencyCoordinate("ch.qos.logback", "logback-core");

	/** A jar whose entries are {@code name, classFileMajor} pairs (a negative major means "not a class file"). */
	private static byte[] jar(Object... nameAndMajor) throws IOException {
		ByteArrayOutputStream out = new ByteArrayOutputStream();
		try (ZipOutputStream zip = new ZipOutputStream(out)) {
			for (int i = 0; i < nameAndMajor.length; i += 2) {
				zip.putNextEntry(new ZipEntry((String) nameAndMajor[i]));
				int major = (Integer) nameAndMajor[i + 1];
				if (major >= 0) {
					zip.write(new byte[] { (byte) 0xCA, (byte) 0xFE, (byte) 0xBA, (byte) 0xBE, 0, 0, (byte) (major >> 8),
							(byte) major, 1, 2, 3 });
				} else {
					zip.write("not a class".getBytes());
				}
				zip.closeEntry();
			}
		}
		return out.toByteArray();
	}

	private static OptionalInt read(byte[] jar) throws IOException {
		return MavenJarJavaRequirementClient.requiredJavaMajor(new ByteArrayInputStream(jar));
	}

	@Test
	void highestClassFileVersionWins() throws Exception {
		assertEquals(OptionalInt.of(11), read(jar("a/A.class", 52, "a/B.class", 55, "a/C.class", 52)));
	}

	@Test
	void java8Library_withJava9ModuleInfoAndMultiReleaseClasses_needsJava8() throws Exception {
		// logback 1.3: classes for Java 8, module-info compiled for Java 9
		byte[] jar = jar("module-info.class", 53, "META-INF/versions/11/a/B.class", 55, "a/A.class", 52,
				"META-INF/MANIFEST.MF", -1);
		assertEquals(OptionalInt.of(8), read(jar));
	}

	@Test
	void noClassFiles_isUnknown() throws Exception {
		assertEquals(OptionalInt.empty(), read(jar("META-INF/MANIFEST.MF", -1, "a/", -1)));
	}

	@Test
	void classFileMajorToJavaVersion() {
		assertEquals(8, MavenJarJavaRequirementClient.javaMajor(52));
		assertEquals(11, MavenJarJavaRequirementClient.javaMajor(55));
		assertEquals(25, MavenJarJavaRequirementClient.javaMajor(69));
		assertEquals(1, MavenJarJavaRequirementClient.javaMajor(45));
	}

	@Test
	void downloadsTheJarFromTheRepository_and404IsUnknown() throws Exception {
		byte[] java11Jar = jar("ch/qos/logback/core/Context.class", 55);
		HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
		server.createContext("/maven2/", exchange -> {
			String path = exchange.getRequestURI().getPath();
			if (path.equals("/maven2/ch/qos/logback/logback-core/1.5.38/logback-core-1.5.38.jar")) {
				exchange.sendResponseHeaders(200, java11Jar.length);
				exchange.getResponseBody().write(java11Jar);
			} else if (path.contains("/9.9.9/")) {
				exchange.sendResponseHeaders(503, -1);
			} else {
				exchange.sendResponseHeaders(404, -1);
			}
			exchange.close();
		});
		server.start();
		try {
			MavenJarJavaRequirementClient client = new MavenJarJavaRequirementClient(
					"http://127.0.0.1:" + server.getAddress().getPort() + "/maven2", Duration.ofSeconds(10));

			assertEquals(OptionalInt.of(11), client.requiredJavaMajor(LOGBACK, "1.5.38"));
			assertEquals(OptionalInt.empty(), client.requiredJavaMajor(LOGBACK, "1.0.0"));
			assertThrows(RepositoryQueryException.class, () -> client.requiredJavaMajor(LOGBACK, "9.9.9"));
		} finally {
			server.stop(0);
		}
	}
}
