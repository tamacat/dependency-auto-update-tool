package org.tamacat.depupdate;

import java.util.ArrayList;
import java.util.List;

/**
 * Hand-rolled {@link Reporter} test double: records every call verbatim for
 * precise assertions, instead of parsing {@link DefaultReporter}'s text
 * output.
 */
final class RecordingReporter implements Reporter {

	record RecordedResult(UpdateCandidate candidate, ValidationResult validationResult, ApplyResult applyResult) {
	}

	final List<DetectionResult> detections = new ArrayList<>();
	final List<RecordedResult> results = new ArrayList<>();
	final List<EnvironmentCheckException> environmentFailures = new ArrayList<>();
	boolean finished = false;

	@Override
	public void recordDetection(DetectionResult detectionResult) {
		detections.add(detectionResult);
	}

	@Override
	public void record(UpdateCandidate candidate, ValidationResult validationResult, ApplyResult applyResult) {
		results.add(new RecordedResult(candidate, validationResult, applyResult));
	}

	@Override
	public void recordEnvironmentFailure(EnvironmentCheckException failure) {
		environmentFailures.add(failure);
	}

	@Override
	public void finish() {
		finished = true;
	}
}
