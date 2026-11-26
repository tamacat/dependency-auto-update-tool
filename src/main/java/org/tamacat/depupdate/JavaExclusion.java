package org.tamacat.depupdate;

import java.util.Objects;

/**
 * Newer versions of a target that Detector left out because their jars need
 * a newer Java than the branch's JDK. Carried on {@link UpdateCandidate} and
 * {@code NoUpdateAvailable} so Reporter can say why a newer version was not
 * proposed.
 *
 * @param fromVersion the oldest left-out version; every newer version is left
 *        out too (the Java a library needs is assumed not to go down as its
 *        version goes up)
 * @param requiredJavaMajor the Java version {@code fromVersion} needs
 * @param branchJavaMajor the branch's JDK ({@code branch.<name>.expectedJdkMajor})
 */
public record JavaExclusion(String fromVersion, int requiredJavaMajor, int branchJavaMajor) {

	public JavaExclusion {
		Objects.requireNonNull(fromVersion, "fromVersion");
	}

	/** For the report, e.g. {@code 1.4.0 and newer need Java 11 (branch JDK 8)}. */
	public String describe() {
		return fromVersion + " and newer need Java " + requiredJavaMajor + " (branch JDK " + branchJavaMajor + ")";
	}
}
