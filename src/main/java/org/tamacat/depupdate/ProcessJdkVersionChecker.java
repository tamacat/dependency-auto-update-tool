package org.tamacat.depupdate;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;
import java.util.OptionalInt;
import java.util.concurrent.TimeUnit;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Real {@link JdkVersionChecker}: launches {@code <jdkHome>/bin/java -version}
 * (or {@code java.exe} on Windows) as a subprocess and parses its output.
 */
public final class ProcessJdkVersionChecker implements JdkVersionChecker {

	private static final Pattern VERSION_PATTERN = Pattern.compile("version \"(\\d+)(?:\\.(\\d+))?");
	private static final int PROBE_TIMEOUT_SECONDS = 15;

	@Override
	public OptionalInt detectMajorVersion(Path jdkHome) {
		if (jdkHome == null) {
			return OptionalInt.empty();
		}
		Path javaBinary = resolveJavaBinary(jdkHome);
		if (!Files.isRegularFile(javaBinary)) {
			return OptionalInt.empty();
		}
		try {
			ProcessBuilder pb = new ProcessBuilder(javaBinary.toString(), "-version");
			pb.redirectErrorStream(true);
			Process process = pb.start();
			String output = new String(process.getInputStream().readAllBytes(), StandardCharsets.UTF_8);
			boolean finished = process.waitFor(PROBE_TIMEOUT_SECONDS, TimeUnit.SECONDS);
			if (!finished) {
				process.destroyForcibly();
				return OptionalInt.empty();
			}
			return parseMajorVersion(output);
		} catch (IOException e) {
			return OptionalInt.empty();
		} catch (InterruptedException e) {
			Thread.currentThread().interrupt();
			return OptionalInt.empty();
		}
	}

	static Path resolveJavaBinary(Path jdkHome) {
		boolean windows = System.getProperty("os.name", "").toLowerCase(Locale.ROOT).contains("win");
		return jdkHome.resolve("bin").resolve(windows ? "java.exe" : "java");
	}

	/**
	 * Parses the major version out of {@code java -version}'s output, e.g.
	 * {@code version "25.0.1"} -&gt; 25, or the old {@code version "1.8.0_462"}
	 * scheme -&gt; 8.
	 */
	static OptionalInt parseMajorVersion(String versionOutput) {
		if (versionOutput == null) {
			return OptionalInt.empty();
		}
		Matcher m = VERSION_PATTERN.matcher(versionOutput);
		if (!m.find()) {
			return OptionalInt.empty();
		}
		String first = m.group(1);
		String second = m.group(2);
		if ("1".equals(first) && second != null) {
			// Old-style version scheme: "1.8.0_462" -> major version 8.
			return OptionalInt.of(Integer.parseInt(second));
		}
		return OptionalInt.of(Integer.parseInt(first));
	}
}
