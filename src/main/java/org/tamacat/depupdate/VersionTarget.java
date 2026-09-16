package org.tamacat.depupdate;

import java.util.List;
import java.util.Objects;

/**
 * The unit Detector enumerates from {@code pom.xml} and Validator/Applier
 * operate on -- a closed set of two variants (domain-entities.md).
 *
 * <p>Grouping rule (business-rules.md BR-1a step 1): two dependencies belong
 * to the same {@link PropertyIndirectedTarget} if and only if their
 * {@code <version>} text is the identical {@code ${propertyName}} reference.
 * A dependency with a literal version is always a {@link DirectDependencyTarget},
 * never grouped with anything.
 */
public sealed interface VersionTarget permits DirectDependencyTarget, PropertyIndirectedTarget {

	/**
	 * The literal, currently-declared version value -- for a
	 * {@link PropertyIndirectedTarget} this is the property's current
	 * resolved value, not the {@code ${...}} reference text.
	 */
	String currentVersion();
}

/**
 * A dependency whose {@code <version>} is a literal string. The common case.
 */
record DirectDependencyTarget(DependencyCoordinate coordinate, String currentVersion) implements VersionTarget {

	DirectDependencyTarget {
		Objects.requireNonNull(coordinate, "coordinate");
		if (currentVersion == null || currentVersion.isBlank()) {
			throw new IllegalArgumentException("currentVersion must not be blank");
		}
	}
}

/**
 * One or more dependencies whose {@code <version>} is {@code ${propertyName}};
 * {@code currentVersion} is the property's current resolved value, read from
 * {@code <properties>/<propertyName>}. All members move together, atomically,
 * because they share one property.
 *
 * <p>{@code members} always has at least one entry (the real {@code tomcat.version}
 * case in this repository's own {@code pom.xml} has two: {@code tomcat-embed-core},
 * {@code tomcat-embed-jasper}).
 */
record PropertyIndirectedTarget(String propertyName, List<DependencyCoordinate> members, String currentVersion)
		implements VersionTarget {

	PropertyIndirectedTarget {
		if (propertyName == null || propertyName.isBlank()) {
			throw new IllegalArgumentException("propertyName must not be blank");
		}
		if (currentVersion == null || currentVersion.isBlank()) {
			throw new IllegalArgumentException("currentVersion must not be blank");
		}
		members = List.copyOf(Objects.requireNonNull(members, "members"));
		if (members.isEmpty()) {
			throw new IllegalArgumentException("PropertyIndirectedTarget requires at least one member coordinate");
		}
	}
}
