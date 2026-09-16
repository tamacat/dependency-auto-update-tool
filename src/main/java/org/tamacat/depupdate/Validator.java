package org.tamacat.depupdate;

/**
 * Judges whether a candidate update is safe to apply, using a real build,
 * not a heuristic (FR-2.1, components.md).
 */
public interface Validator {

	/**
	 * Temporarily applies the candidate to {@code pom.xml}, runs
	 * {@code mvn clean verify}, classifies the outcome (business-rules.md
	 * BR-5), and reverts the temporary change unless the outcome is
	 * {@code GREEN} (in which case the modified {@code pom.xml} is left in
	 * place for Applier).
	 */
	ValidationResult validate(UpdateCandidate candidate);
}
