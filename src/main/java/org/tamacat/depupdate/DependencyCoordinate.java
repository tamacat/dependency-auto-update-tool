package org.tamacat.depupdate;

/**
 * The key identifying one direct Maven dependency: {@code groupId:artifactId}.
 *
 * <p>Used as the map key in {@link ToolConfiguration}'s per-dependency
 * version-range policy, and nested inside {@link VersionTarget} (either as a
 * {@code DirectDependencyTarget}'s coordinate or a member of a
 * {@code PropertyIndirectedTarget}).
 */
public record DependencyCoordinate(String groupId, String artifactId) {

	public DependencyCoordinate {
		if (groupId == null || groupId.isBlank()) {
			throw new IllegalArgumentException("groupId must not be blank");
		}
		if (artifactId == null || artifactId.isBlank()) {
			throw new IllegalArgumentException("artifactId must not be blank");
		}
	}

	@Override
	public String toString() {
		return groupId + ":" + artifactId;
	}
}
