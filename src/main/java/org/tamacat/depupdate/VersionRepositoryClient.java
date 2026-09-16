package org.tamacat.depupdate;

import java.util.List;

/**
 * Queries a Maven repository directly for a coordinate's available versions,
 * independent of {@code versions-maven-plugin} (Application Design ADR-3).
 */
public interface VersionRepositoryClient {

	/**
	 * @return every version string the repository's metadata lists for this
	 *         coordinate (unfiltered, unordered)
	 * @throws RepositoryQueryException on any network/timeout/parse failure
	 */
	List<String> queryAvailableVersions(DependencyCoordinate coordinate) throws RepositoryQueryException;
}
