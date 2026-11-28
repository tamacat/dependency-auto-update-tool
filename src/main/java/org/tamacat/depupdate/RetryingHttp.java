package org.tamacat.depupdate;

import java.io.IOException;
import java.io.InputStream;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Duration;
import java.util.Optional;

/**
 * Sends a request and, when the server answers that it is busy (429 Too Many
 * Requests, or 502/503/504), waits and sends it again. Maven Central answers
 * 429 to a runner whose address it is throttling for the moment; giving up on
 * the first one fails every dependency of the run.
 *
 * <p>The wait follows {@code Retry-After} when it is given in seconds,
 * otherwise it doubles from {@link #FIRST_BACKOFF}; no single wait is longer
 * than {@link #MAX_WAIT}. After {@link #MAX_ATTEMPTS} attempts the last
 * response is returned as it is, so the caller reports its status.
 */
final class RetryingHttp {

	static final int MAX_ATTEMPTS = 4;
	static final Duration FIRST_BACKOFF = Duration.ofSeconds(2);
	static final Duration MAX_WAIT = Duration.ofSeconds(60);

	/** How the wait between attempts is spent; replaced in tests. */
	interface Sleeper {
		void sleep(Duration duration) throws InterruptedException;
	}

	static final Sleeper THREAD_SLEEP = duration -> Thread.sleep(duration.toMillis());

	private final HttpClient httpClient;
	private final Sleeper sleeper;

	RetryingHttp(HttpClient httpClient, Sleeper sleeper) {
		this.httpClient = httpClient;
		this.sleeper = sleeper;
	}

	<T> HttpResponse<T> send(HttpRequest request, HttpResponse.BodyHandler<T> handler)
			throws IOException, InterruptedException {
		for (int attempt = 1;; attempt++) {
			HttpResponse<T> response = httpClient.send(request, handler);
			if (!isRetryable(response.statusCode()) || attempt == MAX_ATTEMPTS) {
				return response;
			}
			if (response.body() instanceof InputStream body) {
				body.close();
			}
			sleeper.sleep(waitBefore(attempt + 1, response.headers().firstValue("Retry-After")));
		}
	}

	static boolean isRetryable(int status) {
		return status == 429 || status == 502 || status == 503 || status == 504;
	}

	/** The wait before attempt {@code nextAttempt} (2 or more). */
	static Duration waitBefore(int nextAttempt, Optional<String> retryAfter) {
		Duration wait = retryAfter.flatMap(RetryingHttp::seconds)
				.orElse(FIRST_BACKOFF.multipliedBy(1L << (nextAttempt - 2)));
		return wait.compareTo(MAX_WAIT) > 0 ? MAX_WAIT : wait;
	}

	/** {@code Retry-After} in its delay-seconds form; the HTTP-date form is not used. */
	private static Optional<Duration> seconds(String value) {
		try {
			long s = Long.parseLong(value.trim());
			return s >= 0 ? Optional.of(Duration.ofSeconds(s)) : Optional.empty();
		} catch (NumberFormatException e) {
			return Optional.empty();
		}
	}
}
