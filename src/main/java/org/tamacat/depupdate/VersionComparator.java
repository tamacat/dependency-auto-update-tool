package org.tamacat.depupdate;

import java.util.Comparator;

/**
 * Total ordering over version strings, used by Detector to (a) filter a
 * target's available versions down to those strictly newer than its current
 * version, and (b) pick the highest surviving version as the candidate
 * (business-logic-model.md Step 2: "candidateVersion := highest version in
 * inPolicy").
 *
 * <p><strong>Code Generation decision, disclosed</strong>: neither
 * business-rules.md BR-1/BR-1a nor business-logic-model.md's pseudocode
 * spell out how "available versions" is filtered down to versions actually
 * newer than the current one before BR-1's bump classification is applied --
 * they describe classifying and filtering candidates, implicitly assuming
 * only newer versions are under consideration. This comparator fills that
 * gap: it orders by the same leading-numeric-segment scheme BR-1 uses, then
 * -- only when the full numeric tuple is identical -- treats a version with
 * no qualifier as newer than one with a qualifier (a stable release
 * supersedes a pre-release of the same numeric tuple), falling back to a
 * case-insensitive lexical comparison of the qualifiers themselves.
 */
public final class VersionComparator implements Comparator<String> {

	public static final VersionComparator INSTANCE = new VersionComparator();

	@Override
	public int compare(String a, String b) {
		int[] segmentsA = VersionSegments.leadingNumericSegments(a);
		int[] segmentsB = VersionSegments.leadingNumericSegments(b);
		int length = Math.max(segmentsA.length, segmentsB.length);
		for (int i = 0; i < length; i++) {
			int valueA = i < segmentsA.length ? segmentsA[i] : 0;
			int valueB = i < segmentsB.length ? segmentsB[i] : 0;
			if (valueA != valueB) {
				return Integer.compare(valueA, valueB);
			}
		}
		String qualifierA = VersionSegments.qualifier(a);
		String qualifierB = VersionSegments.qualifier(b);
		if (qualifierA.isEmpty() && qualifierB.isEmpty()) {
			return 0;
		}
		if (qualifierA.isEmpty()) {
			return 1;
		}
		if (qualifierB.isEmpty()) {
			return -1;
		}
		return qualifierA.compareToIgnoreCase(qualifierB);
	}
}
