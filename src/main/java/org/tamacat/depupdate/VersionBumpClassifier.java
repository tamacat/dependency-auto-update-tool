package org.tamacat.depupdate;

/**
 * business-rules.md BR-1: version-bump classification (patch / minor /
 * major), via a hand-rolled numeric-tuple comparison -- deliberately not
 * {@code org.apache.maven.artifact.versioning.ComparableVersion}, since this
 * tool does not run inside Maven's own plugin classloader and pulling in
 * {@code maven-artifact} would be a new dependency this rule avoids.
 *
 * <p><strong>Known limitation, deliberately accepted</strong> (BR-1): this
 * numeric-tuple heuristic can misclassify a candidate for a library that
 * doesn't follow semver-like conventions. This only decides which candidates
 * are <em>attempted</em>; the real build/test in Validator (BR-4) is the
 * actual safety gate regardless of classification.
 */
public final class VersionBumpClassifier {

	private VersionBumpClassifier() {
	}

	/**
	 * Compares the leading numeric segments of {@code currentVersion} and
	 * {@code candidateVersion} position by position:
	 * <ul>
	 *   <li>1st segment differs -&gt; {@link VersionBump#MAJOR}</li>
	 *   <li>1st equal, 2nd differs (or absent in one, present in the other) -&gt; {@link VersionBump#MINOR}</li>
	 *   <li>1st and 2nd equal, anything else differs -&gt; {@link VersionBump#PATCH}</li>
	 * </ul>
	 */
	public static VersionBump classify(String currentVersion, String candidateVersion) {
		int[] current = VersionSegments.leadingNumericSegments(currentVersion);
		int[] candidate = VersionSegments.leadingNumericSegments(candidateVersion);

		Integer currentMajor = segmentAt(current, 0);
		Integer candidateMajor = segmentAt(candidate, 0);
		if (!java.util.Objects.equals(currentMajor, candidateMajor)) {
			return VersionBump.MAJOR;
		}

		Integer currentMinor = segmentAt(current, 1);
		Integer candidateMinor = segmentAt(candidate, 1);
		if (!java.util.Objects.equals(currentMinor, candidateMinor)) {
			return VersionBump.MINOR;
		}

		return VersionBump.PATCH;
	}

	private static Integer segmentAt(int[] segments, int index) {
		return index < segments.length ? segments[index] : null;
	}
}
