package org.tamacat.depupdate;

import java.nio.file.Path;
import java.util.OptionalInt;

/**
 * Detects the major version reported by a specific JDK installation, by
 * invoking {@code <jdkHome>/bin/java -version} in a subprocess (business-rules.md
 * BR-3) -- never the tool's own running JVM. Extracted as an interface so
 * Detector's pre-flight logic can be unit-tested without spawning real
 * processes.
 */
public interface JdkVersionChecker {

	/**
	 * @return the major version reported by the JDK at {@code jdkHome}, or
	 *         empty if the path does not contain a runnable {@code java}
	 *         binary (missing, not executable, or the probe failed/timed
	 *         out)
	 */
	OptionalInt detectMajorVersion(Path jdkHome);
}
