package org.tamacat.depupdate;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.Locale;
import java.util.Map;

/**
 * Real {@link BuildRunner}: shells out to {@code mvn clean verify}
 * (business-rules.md BR-4 step 2) with {@code JAVA_HOME} explicitly set to
 * the candidate branch's configured JDK home for the subprocess only --
 * never relying on the tool's own ambient {@code JAVA_HOME}/PATH
 * (tech-stack-decisions.md).
 */
public final class ProcessBuildRunner implements BuildRunner {

	@Override
	public BuildResult run(Path workingDirectory, Path jdkHome) throws IOException, InterruptedException {
		boolean windows = System.getProperty("os.name", "").toLowerCase(Locale.ROOT).contains("win");
		String mvnExecutable = windows ? "mvn.cmd" : "mvn";

		// "clean" first, always -- guarantees a fresh target/ for every
		// candidate, which BR-5's output-only classification depends on being
		// trustworthy (target/ is gitignored, so nothing outside this
		// subprocess's own run can leave stale build output behind).
		ProcessBuilder pb = new ProcessBuilder(mvnExecutable, "clean", "verify");
		pb.directory(workingDirectory.toFile());
		pb.redirectErrorStream(true);

		Map<String, String> env = pb.environment();
		env.put("JAVA_HOME", jdkHome.toString());
		String jdkBin = jdkHome.resolve("bin").toString();
		String existingPath = env.getOrDefault("PATH", "");
		env.put("PATH", jdkBin + File.pathSeparator + existingPath);

		Process process = pb.start();
		String output = new String(process.getInputStream().readAllBytes(), StandardCharsets.UTF_8);
		int exitCode = process.waitFor();
		return new BuildResult(exitCode, output);
	}
}
