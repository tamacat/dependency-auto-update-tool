package org.tamacat.depupdate;

/** CLI entry point. See {@code tools/dependency-auto-update/README.md} for usage. */
public final class Main {

	private Main() {
	}

	public static void main(String[] args) {
		int exitCode = new Driver().run(args);
		System.exit(exitCode);
	}
}
