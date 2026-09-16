package org.tamacat.depupdate;

import java.io.IOException;
import java.nio.file.Path;

/**
 * Runs {@code mvn clean verify} against a working directory under a specific
 * JDK home. Extracted as an interface so {@link DefaultValidator} can be
 * unit-tested without spawning a real Maven build.
 */
public interface BuildRunner {

	BuildResult run(Path workingDirectory, Path jdkHome) throws IOException, InterruptedException;
}

/**
 * @param exitCode the subprocess's exit code (0 == success)
 * @param combinedOutput stdout and stderr, interleaved as the process produced them
 */
record BuildResult(int exitCode, String combinedOutput) {
}
