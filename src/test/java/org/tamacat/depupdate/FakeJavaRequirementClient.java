package org.tamacat.depupdate;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.OptionalInt;

/**
 * Hand-rolled {@link JavaRequirementClient} test double: versions from a given
 * one onwards need a given Java, everything older needs Java 8.
 */
final class FakeJavaRequirementClient implements JavaRequirementClient {

	private record Step(String fromVersion, int java) {
	}

	private final Map<DependencyCoordinate, List<Step>> steps = new HashMap<>();
	private final Map<DependencyCoordinate, String> failures = new HashMap<>();
	final List<String> queried = new ArrayList<>();

	/** {@code version} and every newer version of {@code coordinate} need Java {@code java}. */
	FakeJavaRequirementClient needs(DependencyCoordinate coordinate, String version, int java) {
		steps.computeIfAbsent(coordinate, k -> new ArrayList<>()).add(new Step(version, java));
		return this;
	}

	FakeJavaRequirementClient failing(DependencyCoordinate coordinate, String diagnostic) {
		failures.put(coordinate, diagnostic);
		return this;
	}

	@Override
	public OptionalInt requiredJavaMajor(DependencyCoordinate coordinate, String version)
			throws RepositoryQueryException {
		queried.add(coordinate.artifactId() + ":" + version);
		if (failures.containsKey(coordinate)) {
			throw new RepositoryQueryException(failures.get(coordinate));
		}
		int java = 8;
		for (Step s : steps.getOrDefault(coordinate, List.of())) {
			if (VersionComparator.INSTANCE.compare(version, s.fromVersion()) >= 0) {
				java = Math.max(java, s.java());
			}
		}
		return OptionalInt.of(java);
	}
}
