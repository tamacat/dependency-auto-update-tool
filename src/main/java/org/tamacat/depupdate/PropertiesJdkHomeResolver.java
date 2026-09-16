package org.tamacat.depupdate;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Optional;
import java.util.Properties;

/**
 * Reads the machine-local, gitignored JDK-home mapping from a
 * {@code .properties} file (Code Generation's own format choice, per
 * tech-stack-decisions.md's deferral). Keys look like
 * {@code jdkHome.<branch>=<path>}, e.g. {@code jdkHome.master=/opt/jdk8}.
 *
 * <p>A missing file is not an error here -- it simply resolves nothing for
 * every branch, which {@code Detector.validateEnvironment()} then reports as
 * an actionable {@code BRANCH_JDK_MISMATCH} ("unset/missing") rather than a
 * crash.
 */
public final class PropertiesJdkHomeResolver implements JdkHomeResolver {

	private static final String KEY_PREFIX = "jdkHome.";

	private final Map<String, Path> jdkHomesByBranch;

	PropertiesJdkHomeResolver(Map<String, Path> jdkHomesByBranch) {
		this.jdkHomesByBranch = Map.copyOf(jdkHomesByBranch);
	}

	public static PropertiesJdkHomeResolver load(Path path) throws IOException {
		Map<String, Path> map = new LinkedHashMap<>();
		if (Files.exists(path)) {
			Properties props = new Properties();
			try (InputStream in = Files.newInputStream(path)) {
				props.load(in);
			}
			for (String name : props.stringPropertyNames()) {
				if (!name.startsWith(KEY_PREFIX)) {
					continue;
				}
				String branch = name.substring(KEY_PREFIX.length());
				String value = props.getProperty(name);
				if (value != null && !value.isBlank()) {
					map.put(branch, Path.of(value.trim()));
				}
			}
		}
		return new PropertiesJdkHomeResolver(map);
	}

	@Override
	public Optional<Path> resolve(String branch) {
		return Optional.ofNullable(jdkHomesByBranch.get(branch));
	}
}
