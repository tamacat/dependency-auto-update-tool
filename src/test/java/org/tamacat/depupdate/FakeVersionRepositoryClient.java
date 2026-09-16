package org.tamacat.depupdate;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

/** Hand-rolled {@link VersionRepositoryClient} test double backed by an in-memory map. */
final class FakeVersionRepositoryClient implements VersionRepositoryClient {

	private final Map<DependencyCoordinate, List<String>> versionsByCoordinate = new HashMap<>();
	private final Map<DependencyCoordinate, String> failuresByCoordinate = new HashMap<>();
	int queryCount = 0;

	FakeVersionRepositoryClient withVersions(DependencyCoordinate coordinate, String... versions) {
		versionsByCoordinate.put(coordinate, List.of(versions));
		return this;
	}

	FakeVersionRepositoryClient withFailure(DependencyCoordinate coordinate, String diagnostic) {
		failuresByCoordinate.put(coordinate, diagnostic);
		return this;
	}

	@Override
	public List<String> queryAvailableVersions(DependencyCoordinate coordinate) throws RepositoryQueryException {
		queryCount++;
		if (failuresByCoordinate.containsKey(coordinate)) {
			throw new RepositoryQueryException(failuresByCoordinate.get(coordinate));
		}
		return versionsByCoordinate.getOrDefault(coordinate, List.of());
	}
}
