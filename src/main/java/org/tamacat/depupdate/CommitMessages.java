package org.tamacat.depupdate;

/** Builds the commit message for one applied {@link UpdateCandidate}. */
final class CommitMessages {

	private CommitMessages() {
	}

	static String forCandidate(UpdateCandidate candidate) {
		String what = describe(candidate.target());
		String bump = " " + what + " from " + candidate.currentVersion() + " to " + candidate.candidateVersion();
		SecurityStatus security = candidate.security();
		if (!security.fixesSomething()) {
			return "chore(deps): bump" + bump;
		}
		String message = "fix(deps): bump" + bump + "\n\nFixes known vulnerabilities: "
				+ String.join(", ", security.fixed().stream().sorted().toList());
		if (!security.remaining().isEmpty()) {
			message += "\nStill affected by: " + String.join(", ", security.remaining().stream().sorted().toList());
		}
		if (security.escalated()) {
			message += "\nThis is a larger bump than the configured version-range policy, chosen because it fixes"
					+ " the vulnerabilities (securityFixPolicy).";
		}
		return message;
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
