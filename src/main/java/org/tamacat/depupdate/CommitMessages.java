package org.tamacat.depupdate;

/** Builds the commit message for one applied {@link UpdateCandidate}. */
final class CommitMessages {

	private CommitMessages() {
	}

	static String forCandidate(UpdateCandidate candidate) {
		String what = describe(candidate.target());
		return "chore(deps): bump " + what + " from " + candidate.currentVersion() + " to "
				+ candidate.candidateVersion();
	}

	private static String describe(VersionTarget target) {
		if (target instanceof DirectDependencyTarget ddt) {
			return ddt.coordinate().toString();
		}
		if (target instanceof PropertyIndirectedTarget pit) {
			return pit.propertyName();
		}
		return "dependency";
	}
}
