package org.tamacat.depupdate;

/**
 * A repository query for one coordinate's available versions failed
 * (network, timeout, malformed metadata). Caught per-target by Detector and
 * turned into {@code DetectionResult.QueryFailed} -- never propagated up
 * (ADR-10).
 */
public class RepositoryQueryException extends Exception {

	public RepositoryQueryException(String message) {
		super(message);
	}

	public RepositoryQueryException(String message, Throwable cause) {
		super(message, cause);
	}
}
