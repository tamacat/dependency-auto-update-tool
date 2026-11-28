package org.tamacat.depupdate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.OptionalInt;
import java.util.concurrent.atomic.AtomicInteger;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import com.sun.net.httpserver.HttpServer;

/** A busy server (429, 502-504) is asked again after a wait, instead of failing the query at once. */
class RetryingHttpTest {

	private static final DependencyCoordinate WIDGET = new DependencyCoordinate("org.example", "widget");
	private static final String METADATA = "<metadata><versioning><versions><version>1.0.0</version>"
			+ "<version>1.0.1</version></versions></versioning></metadata>";

	private HttpServer server;
	private final List<Duration> waits = new ArrayList<>();
	private final AtomicInteger requests = new AtomicInteger();

	@AfterEach
	void stop() {
		if (server != null) {
			server.stop(0);
		}
	}

	/** A server that answers {@code busy} {@code busyTimes} times, then 200 with {@code body}. */
	private String serve(int busy, int busyTimes, String retryAfter, byte[] body) throws Exception {
		server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
		server.createContext("/maven2/", exchange -> {
			if (requests.incrementAndGet() <= busyTimes) {
				if (retryAfter != null) {
					exchange.getResponseHeaders().add("Retry-After", retryAfter);
				}
				exchange.sendResponseHeaders(busy, -1);
			} else {
				exchange.sendResponseHeaders(200, body.length);
				exchange.getResponseBody().write(body);
			}
			exchange.close();
		});
		server.start();
		return "http://127.0.0.1:" + server.getAddress().getPort() + "/maven2";
	}

	private MavenCentralVersionRepositoryClient versions(String baseUrl) {
		return new MavenCentralVersionRepositoryClient(baseUrl, Duration.ofSeconds(10), waits::add);
	}

	@Test
	void tooManyRequests_isAskedAgain_andTheVersionsAreRead() throws Exception {
		String base = serve(429, 2, null, METADATA.getBytes(StandardCharsets.UTF_8));

		assertEquals(List.of("1.0.0", "1.0.1"), versions(base).queryAvailableVersions(WIDGET));
		assertEquals(3, requests.get());
		assertEquals(List.of(Duration.ofSeconds(2), Duration.ofSeconds(4)), waits);
	}

	@Test
	void retryAfterInSeconds_isFollowed() throws Exception {
		String base = serve(503, 1, "7", METADATA.getBytes(StandardCharsets.UTF_8));

		versions(base).queryAvailableVersions(WIDGET);
		assertEquals(List.of(Duration.ofSeconds(7)), waits);
	}

	@Test
	void stillBusyAfterTheLastAttempt_failsWithItsStatus() throws Exception {
		String base = serve(429, Integer.MAX_VALUE, null, new byte[0]);

		RepositoryQueryException e = assertThrows(RepositoryQueryException.class,
				() -> versions(base).queryAvailableVersions(WIDGET));
		assertTrue(e.getMessage().startsWith("HTTP 429 querying"), e.getMessage());
		assertEquals(RetryingHttp.MAX_ATTEMPTS, requests.get());
		assertEquals(RetryingHttp.MAX_ATTEMPTS - 1, waits.size());
	}

	@Test
	void notFound_isNotAskedAgain() throws Exception {
		String base = serve(404, Integer.MAX_VALUE, null, new byte[0]);

		assertThrows(RepositoryQueryException.class, () -> versions(base).queryAvailableVersions(WIDGET));
		assertEquals(1, requests.get());
		assertTrue(waits.isEmpty());
	}

	@Test
	void jarDownload_isAskedAgainToo() throws Exception {
		// One Java 11 class file
		java.io.ByteArrayOutputStream jar = new java.io.ByteArrayOutputStream();
		try (java.util.zip.ZipOutputStream zip = new java.util.zip.ZipOutputStream(jar)) {
			zip.putNextEntry(new java.util.zip.ZipEntry("a/A.class"));
			zip.write(new byte[] { (byte) 0xCA, (byte) 0xFE, (byte) 0xBA, (byte) 0xBE, 0, 0, 0, 55 });
			zip.closeEntry();
		}
		String base = serve(429, 1, null, jar.toByteArray());

		MavenJarJavaRequirementClient client = new MavenJarJavaRequirementClient(base, Duration.ofSeconds(10),
				waits::add);
		assertEquals(OptionalInt.of(11), client.requiredJavaMajor(WIDGET, "1.0.0"));
		assertEquals(2, requests.get());
	}

	@Test
	void waits_doubleAndAreCapped() {
		assertEquals(Duration.ofSeconds(2), RetryingHttp.waitBefore(2, Optional.empty()));
		assertEquals(Duration.ofSeconds(8), RetryingHttp.waitBefore(4, Optional.empty()));
		assertEquals(RetryingHttp.MAX_WAIT, RetryingHttp.waitBefore(2, Optional.of("3600")));
		assertEquals(Duration.ofSeconds(2),
				RetryingHttp.waitBefore(2, Optional.of("Wed, 21 Oct 2026 07:28:00 GMT")), "HTTP-date is not used");
	}
}
