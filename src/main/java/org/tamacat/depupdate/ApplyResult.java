package org.tamacat.depupdate;

/**
 * A closed set of four variants (Application Design ADR-11), produced by
 * {@code Applier.apply()} only for GREEN {@link ValidationResult}s. Every
 * state Applier can actually reach is a distinct, named outcome -- no silent
 * conflation of "no push attempted" and "push attempted and failed" is
 * possible.
 */
public sealed interface ApplyResult permits CommitFailed, CommittedNoPushAttempted, CommittedAndPushed,
		CommittedPushFailed {
}

/**
 * The commit itself failed; {@code pom.xml}'s modification is left in place,
 * no retry (ADR-7).
 */
record CommitFailed(String diagnostic) implements ApplyResult {
}

/** Success, in {@code LOCAL_COMMIT_MANUAL_APPROVAL} mode (the default). */
record CommittedNoPushAttempted(String commitId) implements ApplyResult {
}

/** Success, in {@code FULL_LOOP_AUTO_PUSH} mode. */
record CommittedAndPushed(String commitId) implements ApplyResult {
}

/**
 * The commit succeeded but the subsequent push failed; the commit is
 * durable, the maintainer must push manually.
 */
record CommittedPushFailed(String commitId, String diagnostic) implements ApplyResult {
}
