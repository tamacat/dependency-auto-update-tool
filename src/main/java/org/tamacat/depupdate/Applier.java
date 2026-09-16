package org.tamacat.depupdate;

/**
 * Finalizes a validated-safe ({@code GREEN}) update into git, per the
 * configured commit/push mode (components.md).
 */
public interface Applier {

	/**
	 * Commits the already-modified {@code pom.xml} (left by a GREEN
	 * {@link ValidationResult}), then, only in {@code FULL_LOOP_AUTO_PUSH}
	 * mode and only after a successful commit, attempts to push it. Never
	 * throws -- every distinct outcome is a distinct {@link ApplyResult}
	 * variant (ADR-11).
	 *
	 * @throws IllegalArgumentException if {@code greenResult} is not GREEN
	 */
	ApplyResult apply(ValidationResult greenResult, CommitPushMode mode);
}
