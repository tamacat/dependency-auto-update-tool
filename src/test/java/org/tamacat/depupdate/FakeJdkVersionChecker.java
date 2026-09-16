package org.tamacat.depupdate;

import java.nio.file.Path;
import java.util.OptionalInt;

/** Hand-rolled {@link JdkVersionChecker} test double: returns a fixed, configurable answer. */
final class FakeJdkVersionChecker implements JdkVersionChecker {

	private final OptionalInt result;

	FakeJdkVersionChecker(OptionalInt result) {
		this.result = result;
	}

	static FakeJdkVersionChecker returning(int majorVersion) {
		return new FakeJdkVersionChecker(OptionalInt.of(majorVersion));
	}

	static FakeJdkVersionChecker unresolvable() {
		return new FakeJdkVersionChecker(OptionalInt.empty());
	}

	@Override
	public OptionalInt detectMajorVersion(Path jdkHome) {
		return result;
	}
}
