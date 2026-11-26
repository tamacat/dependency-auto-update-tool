package org.tamacat.depupdate;

import java.util.OptionalInt;

/**
 * Finds the Java version a dependency's jar needs, from the class files it
 * contains -- so that a candidate built for a newer Java than the target
 * branch's JDK is never proposed. A passing {@code mvn clean verify} does not
 * prove this: a library loaded only at run time (an SLF4J provider, for
 * example) can fail to load while every test still passes.
 */
public interface JavaRequirementClient {

	/**
	 * @return the lowest Java major version (8, 11, 17, ...) that can load
	 *         every class in this version's jar, or empty when it cannot be
	 *         told (no jar in the repository, or no class files in it)
	 * @throws RepositoryQueryException on a network/timeout/read failure
	 */
	OptionalInt requiredJavaMajor(DependencyCoordinate coordinate, String version) throws RepositoryQueryException;
}
