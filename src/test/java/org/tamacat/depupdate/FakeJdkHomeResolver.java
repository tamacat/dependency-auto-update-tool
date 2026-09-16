package org.tamacat.depupdate;

import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;
import java.util.Optional;

/** Hand-rolled {@link JdkHomeResolver} test double backed by an in-memory map. */
final class FakeJdkHomeResolver implements JdkHomeResolver {

	private final Map<String, Path> byBranch = new HashMap<>();

	FakeJdkHomeResolver with(String branch, Path path) {
		byBranch.put(branch, path);
		return this;
	}

	@Override
	public Optional<Path> resolve(String branch) {
		return Optional.ofNullable(byBranch.get(branch));
	}
}
