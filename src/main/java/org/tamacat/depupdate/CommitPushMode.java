package org.tamacat.depupdate;

/**
 * The global, invocation-level commit/push behavior (FR-3.2), read from
 * {@link ToolConfiguration} by the top-level driver and passed into every
 * {@code Applier.apply()} call.
 */
public enum CommitPushMode {
	/** Default: stop after a successful local commit; the maintainer decides push/PR. */
	LOCAL_COMMIT_MANUAL_APPROVAL,
	/** Opt-in: also push/publish automatically once the commit succeeds. */
	FULL_LOOP_AUTO_PUSH
}
