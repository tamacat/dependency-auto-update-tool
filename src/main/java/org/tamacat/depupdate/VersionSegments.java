package org.tamacat.depupdate;

import java.util.ArrayList;
import java.util.List;

/**
 * Shared parsing helper for business-rules.md BR-1's version-comparison
 * scheme: split on {@code .}/{@code -} and take the leading run of
 * purely-numeric segments (stop at the first non-numeric segment, which
 * becomes the qualifier).
 */
final class VersionSegments {

	private VersionSegments() {
	}

	static int[] leadingNumericSegments(String version) {
		String[] tokens = version.split("[.\\-]");
		List<Integer> numbers = new ArrayList<>();
		for (String token : tokens) {
			if (token.matches("\\d+")) {
				numbers.add(Integer.parseInt(token));
			} else {
				break;
			}
		}
		int[] result = new int[numbers.size()];
		for (int i = 0; i < numbers.size(); i++) {
			result[i] = numbers.get(i);
		}
		return result;
	}

	/** The remainder of the version string after the leading numeric run, e.g. "RC1" for "1.0-RC1". */
	static String qualifier(String version) {
		String[] tokens = version.split("[.\\-]");
		int numericCount = 0;
		for (String token : tokens) {
			if (token.matches("\\d+")) {
				numericCount++;
			} else {
				break;
			}
		}
		StringBuilder qualifier = new StringBuilder();
		for (int i = numericCount; i < tokens.length; i++) {
			if (qualifier.length() > 0) {
				qualifier.append('-');
			}
			qualifier.append(tokens[i]);
		}
		return qualifier.toString();
	}
}
