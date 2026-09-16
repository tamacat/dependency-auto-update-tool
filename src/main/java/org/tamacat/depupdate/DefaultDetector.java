package org.tamacat.depupdate;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.OptionalInt;
import java.util.Set;

/**
 * Real {@link Detector}: pre-flight (branch/JDK, then clean-working-tree --
 * ADR-7), then {@code pom.xml} parsing, BR-1a grouping, per-target
 * repository queries, and BR-1/BR-2 classification.
 */
public final class DefaultDetector implements Detector {

	private final Path pomXmlPath;
	private final ToolConfiguration configuration;
	private final JdkHomeResolver jdkHomeResolver;
	private final JdkVersionChecker jdkVersionChecker;
	private final VersionRepositoryClient repositoryClient;
	private final GitClient gitClient;

	private volatile Path resolvedJdkHome;

	public DefaultDetector(Path pomXmlPath, ToolConfiguration configuration, JdkHomeResolver jdkHomeResolver,
			JdkVersionChecker jdkVersionChecker, VersionRepositoryClient repositoryClient, GitClient gitClient) {
		this.pomXmlPath = pomXmlPath;
		this.configuration = configuration;
		this.jdkHomeResolver = jdkHomeResolver;
		this.jdkVersionChecker = jdkVersionChecker;
		this.repositoryClient = repositoryClient;
		this.gitClient = gitClient;
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
					"n/a", "unknown branch — not one of master/v2.0-tc11");
		}

		OptionalInt expected = BranchJdkBaseline.expectedMajorVersion(branch);
		if (expected.isEmpty()) {
			throw new BranchJdkMismatchException(branch, "n/a", "unknown branch — not one of master/v2.0-tc11");
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
		return results;
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
					.toList();
			List<String> inPolicy = newer.stream()
					.filter(v -> policy.allows(VersionBumpClassifier.classify(current, v)))
					.toList();

			if (inPolicy.isEmpty()) {
				return new NoUpdateAvailable(target);
			}
			String candidateVersion = inPolicy.stream().max(VersionComparator.INSTANCE).orElseThrow();
			VersionClassification classification = PreReleaseClassifier.classify(candidateVersion);
			return new CandidateFound(new UpdateCandidate(target, current, candidateVersion, classification));
		} catch (RepositoryQueryException e) {
			return new QueryFailed(target, e.getMessage());
		}
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
