package org.tamacat.depupdate;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.OptionalInt;
import java.util.Set;

/**
 * Real {@link Detector}: pre-flight (branch/JDK, then clean-working-tree --
 * ADR-7), then {@code pom.xml} parsing, BR-1a grouping, per-target
 * repository queries, and BR-1/BR-2 classification.
 *
 * <p>Security fixes: when a {@link VulnerabilityClient} is wired in and
 * {@code securityFixPolicy} is not {@code OFF}, the current version of each
 * target is looked up for known vulnerabilities. If it has some and the normal
 * policy's best candidate does not fix them, the smallest series that fixes
 * all of them within the security-fix ceiling is chosen instead (its newest
 * fixing version). Security-fix candidates are returned first, so they are
 * validated and committed before anything else.
 */
public final class DefaultDetector implements Detector {

	private final Path pomXmlPath;
	private final ToolConfiguration configuration;
	private final JdkHomeResolver jdkHomeResolver;
	private final JdkVersionChecker jdkVersionChecker;
	private final VersionRepositoryClient repositoryClient;
	private final GitClient gitClient;
	/** {@code null} when vulnerabilities are not looked up. */
	private final VulnerabilityClient vulnerabilityClient;

	private volatile Path resolvedJdkHome;

	/** A detector that does not look up vulnerabilities. */
	public DefaultDetector(Path pomXmlPath, ToolConfiguration configuration, JdkHomeResolver jdkHomeResolver,
			JdkVersionChecker jdkVersionChecker, VersionRepositoryClient repositoryClient, GitClient gitClient) {
		this(pomXmlPath, configuration, jdkHomeResolver, jdkVersionChecker, repositoryClient, gitClient, null);
	}

	public DefaultDetector(Path pomXmlPath, ToolConfiguration configuration, JdkHomeResolver jdkHomeResolver,
			JdkVersionChecker jdkVersionChecker, VersionRepositoryClient repositoryClient, GitClient gitClient,
			VulnerabilityClient vulnerabilityClient) {
		this.pomXmlPath = pomXmlPath;
		this.configuration = configuration;
		this.jdkHomeResolver = jdkHomeResolver;
		this.jdkVersionChecker = jdkVersionChecker;
		this.repositoryClient = repositoryClient;
		this.gitClient = gitClient;
		this.vulnerabilityClient = vulnerabilityClient;
	}

	/**
	 * The JDK home resolved and validated during {@link #validateEnvironment()},
	 * retained for {@code Validator}'s {@code mvn clean verify} invocation
	 * (BR-3/BR-4). Only meaningful after a successful pre-flight call.
	 */
	public Path getResolvedJdkHome() {
		return resolvedJdkHome;
	}

	@Override
	public void validateEnvironment() throws EnvironmentCheckException {
		String branch;
		try {
			branch = gitClient.currentBranch().trim();
		} catch (IOException | InterruptedException e) {
			if (e instanceof InterruptedException) {
				// Finding 6, code-generation review: restore the interrupt status
				// rather than swallowing it.
				Thread.currentThread().interrupt();
			}
			throw new BranchJdkMismatchException("<unknown - branch detection failed: " + e.getMessage() + ">",
					"n/a", "<unknown -- branch detection itself failed>");
		}

		OptionalInt expected = configuration.expectedJdkMajorVersion(branch);
		if (expected.isEmpty()) {
			throw new BranchJdkMismatchException(branch, "n/a",
					"<not configured -- add branch." + branch + ".expectedJdkMajor=<n> to tool.properties "
							+ "if this branch should be supported>");
		}

		Optional<Path> jdkHomeOpt = jdkHomeResolver.resolve(branch);
		if (jdkHomeOpt.isEmpty()) {
			throw new BranchJdkMismatchException(branch, "unset/missing", String.valueOf(expected.getAsInt()));
		}
		Path jdkHome = jdkHomeOpt.get();

		OptionalInt actual = jdkVersionChecker.detectMajorVersion(jdkHome);
		if (actual.isEmpty()) {
			throw new BranchJdkMismatchException(branch,
					"unset/missing (configured path '" + jdkHome + "' did not report a version)",
					String.valueOf(expected.getAsInt()));
		}
		if (actual.getAsInt() != expected.getAsInt()) {
			throw new BranchJdkMismatchException(branch, String.valueOf(actual.getAsInt()),
					String.valueOf(expected.getAsInt()));
		}
		this.resolvedJdkHome = jdkHome;

		String status;
		try {
			status = gitClient.statusPorcelain();
		} catch (IOException | InterruptedException e) {
			if (e instanceof InterruptedException) {
				Thread.currentThread().interrupt();
			}
			throw new DirtyWorkingTreeException("<git status failed: " + e.getMessage() + ">");
		}
		if (!status.isBlank()) {
			throw new DirtyWorkingTreeException(status);
		}
	}

	@Override
	public List<DetectionResult> findCandidates() {
		String pomText = readPomText();
		List<RawDependencyDeclaration> declarations = PomParser.parseDependencies(pomText);
		Map<String, String> properties = PomParser.parseProperties(pomText);
		List<VersionTarget> targets = VersionTargetGrouper.group(declarations, properties);

		List<DetectionResult> results = new ArrayList<>();
		for (VersionTarget target : targets) {
			results.add(detectOne(target));
		}
		// Security fixes first (stable sort: the pom.xml order is kept otherwise), so a
		// later commit failure, which stops the run, cannot keep them from being applied.
		results.sort(Comparator.comparing((DetectionResult r) -> !isSecurityFix(r)));
		return results;
	}

	private static boolean isSecurityFix(DetectionResult r) {
		return r instanceof CandidateFound cf && cf.candidate().security().fixesSomething();
	}

	private static List<DependencyCoordinate> members(VersionTarget target) {
		if (target instanceof DirectDependencyTarget ddt) {
			return List.of(ddt.coordinate());
		}
		return ((PropertyIndirectedTarget) target).members();
	}

	private DetectionResult detectOne(VersionTarget target) {
		try {
			List<String> available;
			VersionRangePolicy policy;
			if (target instanceof DirectDependencyTarget ddt) {
				available = repositoryClient.queryAvailableVersions(ddt.coordinate());
				policy = configuration.versionRangePolicies().getOrDefault(ddt.coordinate(), VersionRangePolicy.PATCH_ONLY);
			} else if (target instanceof PropertyIndirectedTarget pit) {
				available = intersectMemberVersions(pit);
				policy = VersionTargetGrouper.effectivePolicy(pit, configuration.versionRangePolicies());
			} else {
				throw new IllegalStateException("Unknown VersionTarget implementation: " + target.getClass());
			}

			String current = target.currentVersion();
			List<String> newer = available.stream()
					.filter(v -> VersionComparator.INSTANCE.compare(v, current) > 0)
					.sorted(VersionComparator.INSTANCE)
					.toList();
			List<String> inPolicy = newer.stream()
					.filter(v -> policy.allows(VersionBumpClassifier.classify(current, v)))
					.toList();
			String normalBest = inPolicy.isEmpty() ? null : inPolicy.get(inPolicy.size() - 1);

			Optional<VersionRangePolicy> ceiling = configuration.securityFixPolicy().ceiling();
			if (vulnerabilityClient != null && ceiling.isPresent()) {
				return detectWithSecurity(target, current, newer, normalBest, policy, ceiling.get());
			}
			return normalResult(target, current, normalBest, SecurityStatus.NONE);
		} catch (RepositoryQueryException e) {
			return new QueryFailed(target, e.getMessage());
		}
	}

	private static DetectionResult normalResult(VersionTarget target, String current, String candidate,
			SecurityStatus security) {
		if (candidate == null) {
			return new NoUpdateAvailable(target, security);
		}
		return new CandidateFound(
				new UpdateCandidate(target, current, candidate, PreReleaseClassifier.classify(candidate), security));
	}

	/**
	 * Chooses the candidate when the current version may have known
	 * vulnerabilities. A failed lookup is not fatal: the target is handled as
	 * before and the reason is reported.
	 */
	private DetectionResult detectWithSecurity(VersionTarget target, String current, List<String> newer,
			String normalBest, VersionRangePolicy policy, VersionRangePolicy ceiling) {
		VersionRangePolicy allowed = ceiling.isAtLeastAsPermissiveAs(policy) ? ceiling : policy;
		boolean currentIsPreRelease = PreReleaseClassifier.classify(current) == VersionClassification.PRE_RELEASE;
		// Versions a security fix may move to: within the ceiling, and stable unless already on a pre-release
		List<String> pool = newer.stream()
				.filter(v -> allowed.allows(VersionBumpClassifier.classify(current, v)))
				.filter(v -> currentIsPreRelease || PreReleaseClassifier.classify(v) == VersionClassification.STABLE)
				.toList();
		if (pool.size() > OsvVulnerabilityClient.MAX_BATCH - 2) {
			pool = pool.subList(pool.size() - (OsvVulnerabilityClient.MAX_BATCH - 2), pool.size());
		}
		Set<String> versions = new LinkedHashSet<>();
		versions.add(current);
		if (normalBest != null) {
			versions.add(normalBest);
		}
		versions.addAll(pool);

		Map<String, Set<String>> byVersion;
		try {
			byVersion = lookUp(members(target), List.copyOf(versions));
		} catch (RepositoryQueryException e) {
			return normalResult(target, current, normalBest,
					SecurityStatus.lookupFailed("vulnerability lookup failed: " + e.getMessage()));
		}

		Set<String> vulnerable = byVersion.getOrDefault(current, Set.of());
		if (vulnerable.isEmpty()) {
			return normalResult(target, current, normalBest, SecurityStatus.NONE);
		}
		// Of the current vulnerabilities, which ones a version still has
		java.util.function.Function<String, Set<String>> remainingAt = v -> {
			Set<String> r = new LinkedHashSet<>(byVersion.getOrDefault(v, Set.of()));
			r.retainAll(vulnerable);
			return r;
		};

		// 1. The normal candidate already fixes everything: keep it
		if (normalBest != null && remainingAt.apply(normalBest).isEmpty()) {
			return normalResult(target, current, normalBest,
					new SecurityStatus(vulnerable, vulnerable, Set.of(), false, null));
		}
		// 2. The smallest version that fixes everything, then the newest fixing version in its series
		Optional<String> smallestFix = pool.stream().filter(v -> remainingAt.apply(v).isEmpty()).findFirst();
		if (smallestFix.isPresent()) {
			String fix = smallestFix.get();
			String chosen = pool.stream()
					.filter(v -> VersionComparator.INSTANCE.compare(v, fix) >= 0)
					.filter(v -> VersionBumpClassifier.classify(fix, v) == VersionBump.PATCH)
					.filter(v -> remainingAt.apply(v).isEmpty())
					.max(VersionComparator.INSTANCE).orElse(fix);
			boolean escalated = !policy.allows(VersionBumpClassifier.classify(current, chosen));
			return normalResult(target, current, chosen,
					new SecurityStatus(vulnerable, vulnerable, Set.of(), escalated, null));
		}
		// 3. Nothing fixes everything: keep the normal candidate and say what remains
		if (normalBest != null) {
			Set<String> remaining = remainingAt.apply(normalBest);
			Set<String> fixed = new LinkedHashSet<>(vulnerable);
			fixed.removeAll(remaining);
			return normalResult(target, current, normalBest,
					new SecurityStatus(vulnerable, fixed, remaining, false, null));
		}
		return new NoUpdateAvailable(target, new SecurityStatus(vulnerable, Set.of(), vulnerable, false, null));
	}

	/** Vulnerability IDs per version, merged over all members of the target. */
	private Map<String, Set<String>> lookUp(List<DependencyCoordinate> coordinates, List<String> versions)
			throws RepositoryQueryException {
		List<VulnerabilityClient.Query> queries = new ArrayList<>();
		for (DependencyCoordinate c : coordinates) {
			for (String v : versions) {
				queries.add(new VulnerabilityClient.Query(c, v));
			}
		}
		List<Set<String>> answers = vulnerabilityClient.vulnerabilities(queries);
		Map<String, Set<String>> byVersion = new java.util.HashMap<>();
		for (int i = 0; i < queries.size() && i < answers.size(); i++) {
			byVersion.computeIfAbsent(queries.get(i).version(), k -> new LinkedHashSet<>()).addAll(answers.get(i));
		}
		return byVersion;
	}

	private List<String> intersectMemberVersions(PropertyIndirectedTarget target) throws RepositoryQueryException {
		Set<String> intersection = null;
		for (DependencyCoordinate member : target.members()) {
			Set<String> memberVersions = new HashSet<>(repositoryClient.queryAvailableVersions(member));
			if (intersection == null) {
				intersection = memberVersions;
			} else {
				intersection.retainAll(memberVersions);
			}
		}
		return intersection == null ? List.of() : new ArrayList<>(intersection);
	}

	private String readPomText() {
		try {
			return Files.readString(pomXmlPath, StandardCharsets.UTF_8);
		} catch (IOException e) {
			throw new IllegalStateException("Could not read " + pomXmlPath + ": " + e.getMessage(), e);
		}
	}
}
