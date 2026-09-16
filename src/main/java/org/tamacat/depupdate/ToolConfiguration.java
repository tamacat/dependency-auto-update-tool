package org.tamacat.depupdate;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.OptionalInt;
import java.util.Properties;

/**
 * The parsed, in-memory representation of the repo-checked-in configuration
 * file (Application Design Q4: B, extended by Q7: C). Read once per
 * invocation by the top-level driver at startup.
 *
 * <p>File format decision (Code Generation's own choice, per
 * tech-stack-decisions.md's deferral): a plain Java {@code .properties} file
 * -- no new dependency for YAML parsing. See
 * {@code config/tool.properties} and this module's README for the schema.
 *
 * @param commitPushMode global commit/push behavior (FR-3.2)
 * @param versionRangePolicies per-{@link DependencyCoordinate} policy map;
 *        absent entries default to {@code PATCH_ONLY} at the point of use,
 *        not by being materialized into this map
 * @param expectedJdkMajorByBranch the branches this tool is allowed to
 *        operate on and each one's expected JDK major version -- a
 *        repo-wide policy fact (which branches exist and what they target),
 *        distinct from the machine-local fact of where that JDK is actually
 *        installed (see {@link PropertiesJdkHomeResolver}). A branch absent
 *        from this map is not a recognized target for this tool (BR-3);
 *        this tool ships no hardcoded branch list of its own -- every
 *        adopting project declares its own here.
 * @param repositoryBaseUrl the Maven repository base URL to query for
 *        available versions (Application Design ADR-3; defaults to Maven
 *        Central)
 * @param repositoryQueryTimeout per-target repository query timeout
 *        (nfr-requirements/performance-requirements.md: 30-60s)
 * @param reportDirectory where {@link Reporter} writes its structured report
 *        file, relative to the current working directory unless absolute
 */
public record ToolConfiguration(CommitPushMode commitPushMode,
		Map<DependencyCoordinate, VersionRangePolicy> versionRangePolicies,
		Map<String, Integer> expectedJdkMajorByBranch, String repositoryBaseUrl, Duration repositoryQueryTimeout,
		Path reportDirectory) {

	private static final String DEFAULT_REPOSITORY_BASE_URL = "https://repo1.maven.org/maven2/";
	private static final int DEFAULT_TIMEOUT_SECONDS = 45;
	private static final String DEFAULT_REPORT_DIR = "reports";
	private static final String BRANCH_POLICY_PREFIX = "branch.";
	private static final String BRANCH_POLICY_SUFFIX = ".expectedJdkMajor";

	public ToolConfiguration {
		versionRangePolicies = Map.copyOf(versionRangePolicies);
		expectedJdkMajorByBranch = Map.copyOf(expectedJdkMajorByBranch);
	}

	public static ToolConfiguration defaults() {
		return new ToolConfiguration(CommitPushMode.LOCAL_COMMIT_MANUAL_APPROVAL, Map.of(), Map.of(),
				DEFAULT_REPOSITORY_BASE_URL, Duration.ofSeconds(DEFAULT_TIMEOUT_SECONDS), Path.of(DEFAULT_REPORT_DIR));
	}

	/** BR-3: the expected JDK major version for {@code branch}, if this tool is configured to operate on it. */
	public OptionalInt expectedJdkMajorVersion(String branch) {
		Integer version = expectedJdkMajorByBranch.get(branch);
		return version == null ? OptionalInt.empty() : OptionalInt.of(version);
	}

	/** Test/programmatic convenience: returns a copy with one more (or replaced) branch policy entry. */
	public ToolConfiguration withExpectedJdkMajor(String branch, int major) {
		Map<String, Integer> updated = new LinkedHashMap<>(expectedJdkMajorByBranch);
		updated.put(branch, major);
		return new ToolConfiguration(commitPushMode, versionRangePolicies, updated, repositoryBaseUrl,
				repositoryQueryTimeout, reportDirectory);
	}

	/**
	 * Loads the repo-checked-in configuration file. Missing file is not an
	 * error -- {@link #defaults()} apply (a maintainer who has not created
	 * {@code config/tool.properties} yet still gets a safe, working default).
	 */
	public static ToolConfiguration loadFromProperties(Path path) throws IOException {
		if (!Files.exists(path)) {
			return defaults();
		}
		Properties props = new Properties();
		try (InputStream in = Files.newInputStream(path)) {
			props.load(in);
		}

		CommitPushMode mode = CommitPushMode.LOCAL_COMMIT_MANUAL_APPROVAL;
		String modeStr = props.getProperty("commitPushMode");
		if (modeStr != null && !modeStr.isBlank()) {
			try {
				mode = CommitPushMode.valueOf(modeStr.trim());
			} catch (IllegalArgumentException e) {
				throw new IOException("Invalid commitPushMode value '" + modeStr + "' in " + path
						+ " (expected LOCAL_COMMIT_MANUAL_APPROVAL or FULL_LOOP_AUTO_PUSH)", e);
			}
		}

		String repoUrl = props.getProperty("repositoryBaseUrl", DEFAULT_REPOSITORY_BASE_URL);

		int timeoutSeconds = DEFAULT_TIMEOUT_SECONDS;
		String timeoutStr = props.getProperty("repositoryQueryTimeoutSeconds");
		if (timeoutStr != null && !timeoutStr.isBlank()) {
			try {
				timeoutSeconds = Integer.parseInt(timeoutStr.trim());
			} catch (NumberFormatException e) {
				throw new IOException(
						"Invalid repositoryQueryTimeoutSeconds value '" + timeoutStr + "' in " + path, e);
			}
		}

		String reportDir = props.getProperty("reportDir", DEFAULT_REPORT_DIR);

		Map<DependencyCoordinate, VersionRangePolicy> policies = new LinkedHashMap<>();
		for (String name : props.stringPropertyNames()) {
			if (!name.startsWith("policy.")) {
				continue;
			}
			String coordinateKey = name.substring("policy.".length());
			int sep = coordinateKey.lastIndexOf(':');
			if (sep < 0) {
				throw new IOException("Invalid policy key '" + name + "' in " + path
						+ " (expected policy.<groupId>:<artifactId>=PATCH_ONLY|MINOR|MAJOR -- if you meant to write a"
						+ " groupId:artifactId here, the ':' must be escaped as '\\:' in a .properties file, or"
						+ " java.util.Properties consumes it as the key/value separator before this parser ever sees it)");
			}
			String groupId = coordinateKey.substring(0, sep);
			String artifactId = coordinateKey.substring(sep + 1);
			String value = props.getProperty(name).trim();
			VersionRangePolicy policy;
			try {
				policy = VersionRangePolicy.valueOf(value);
			} catch (IllegalArgumentException e) {
				throw new IOException(
						"Invalid version-range policy '" + value + "' for " + coordinateKey + " in " + path, e);
			}
			policies.put(new DependencyCoordinate(groupId, artifactId), policy);
		}

		Map<String, Integer> branchPolicies = new LinkedHashMap<>();
		for (String name : props.stringPropertyNames()) {
			if (!name.startsWith(BRANCH_POLICY_PREFIX) || !name.endsWith(BRANCH_POLICY_SUFFIX)
					|| name.length() <= BRANCH_POLICY_PREFIX.length() + BRANCH_POLICY_SUFFIX.length()) {
				continue;
			}
			// Trim the known prefix/suffix rather than splitting on '.' -- a branch
			// name may itself contain dots (e.g. "v2.0-tc11").
			String branch = name.substring(BRANCH_POLICY_PREFIX.length(), name.length() - BRANCH_POLICY_SUFFIX.length());
			String value = props.getProperty(name).trim();
			try {
				branchPolicies.put(branch, Integer.parseInt(value));
			} catch (NumberFormatException e) {
				throw new IOException("Invalid expectedJdkMajor value '" + value + "' for branch '" + branch
						+ "' in " + path + " (expected " + BRANCH_POLICY_PREFIX + "<branchName>" + BRANCH_POLICY_SUFFIX
						+ "=<integer>)", e);
			}
		}

		return new ToolConfiguration(mode, policies, branchPolicies, repoUrl, Duration.ofSeconds(timeoutSeconds),
				Path.of(reportDir));
	}
}
