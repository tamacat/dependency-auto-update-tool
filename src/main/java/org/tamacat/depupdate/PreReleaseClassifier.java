package org.tamacat.depupdate;

import java.util.Locale;
import java.util.regex.Pattern;

/**
 * business-rules.md BR-2: pre-release / stable classification. A candidate
 * version is classified {@code PRE_RELEASE} if, case-insensitively, it
 * contains any of: {@code SNAPSHOT}, {@code alpha}, {@code beta}, an
 * {@code rc} qualifier followed by a digit, or a Maven-style milestone
 * qualifier ({@code -M} followed by a digit). Otherwise {@code STABLE}.
 */
public final class PreReleaseClassifier {

	// Matches "-rc1", ".rc1", or "RC1" with no separator, case-insensitively.
	private static final Pattern RC_PATTERN = Pattern.compile("(?i)[.-]?rc\\d");
	// Matches "-M1" (Maven-style milestone qualifier), case-insensitively.
	private static final Pattern MILESTONE_PATTERN = Pattern.compile("(?i)-m\\d");

	private PreReleaseClassifier() {
	}

	public static VersionClassification classify(String candidateVersion) {
		String lower = candidateVersion.toLowerCase(Locale.ROOT);
		if (lower.contains("snapshot") || lower.contains("alpha") || lower.contains("beta")) {
			return VersionClassification.PRE_RELEASE;
		}
		if (RC_PATTERN.matcher(candidateVersion).find() || MILESTONE_PATTERN.matcher(candidateVersion).find()) {
			return VersionClassification.PRE_RELEASE;
		}
		return VersionClassification.STABLE;
	}
}
