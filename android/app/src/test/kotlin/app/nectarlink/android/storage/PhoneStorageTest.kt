// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.storage

import app.nectarlink.core.StorageFailure
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.File
import java.nio.file.Files

class PhoneStorageTest {
    @Test
    fun validatesRelativeNamesAndPaths() {
        assertTrue(PhoneStorage.isValidName("notes.txt"))
        assertTrue(PhoneStorage.isValidName(".hidden"))
        assertFalse(PhoneStorage.isValidName(""))
        assertFalse(PhoneStorage.isValidName("."))
        assertFalse(PhoneStorage.isValidName(".."))
        assertFalse(PhoneStorage.isValidName("a/b"))
        assertFalse(PhoneStorage.isValidName("a\\b"))
        assertFalse(PhoneStorage.isValidName("bad\u0000name"))

        assertTrue(PhoneStorage.isValidDirPath(""))
        assertTrue(PhoneStorage.isValidDirPath("DCIM/Camera"))
        assertFalse(PhoneStorage.isValidPath(""))
        assertTrue(PhoneStorage.isValidPath("DCIM/Camera/IMG_001.jpg"))
        assertFalse(PhoneStorage.isValidPath("/etc/passwd"))
        assertFalse(PhoneStorage.isValidPath("DCIM/../secret"))
        assertFalse(PhoneStorage.isValidPath("DCIM/Camera/"))
        assertFalse(PhoneStorage.isValidPath("DCIM//Camera"))
    }

    @Test
    fun movesWithinGrantedTreeUsingOneProviderOperationAndActualResult() {
        val provider = MoveProvider()
        moveSafDocument(movePath("Documents/note.txt"), movePath("Archive/note.txt"), provider)

        assertEquals(listOf(Triple("note", "documents", "archive")), provider.moves)
        assertEquals(2, provider.grantChecks)
        assertEquals(null, provider.child("documents", "note.txt"))
        assertEquals("moved-note", provider.child("archive", "note.txt")?.id)
    }

    @Test
    fun refusesUnsafeMovesBeforeDispatchAndDoesNotRetryUnconfirmedResults() {
        fun refused(
            provider: MoveProvider = MoveProvider(),
            from: SafMovePath = movePath("Documents/note.txt"),
            to: SafMovePath = movePath("Archive/note.txt"),
            expected: Class<out StorageFailure>,
            dispatched: Int = 0,
        ) {
            try {
                moveSafDocument(from, to, provider)
                fail("expected ${expected.simpleName}")
            } catch (failure: StorageFailure) {
                assertTrue("unexpected ${failure.javaClass.simpleName}", expected.isInstance(failure))
            }
            assertEquals(dispatched, provider.moves.size)
        }

        refused(provider = MoveProvider().apply { granted = false }, expected = StorageFailure.Denied::class.java)
        refused(provider = MoveProvider().apply { revokeBeforeMove = true }, expected = StorageFailure.Denied::class.java)
        refused(provider = MoveProvider().apply {
            entries["documents" to "note.txt"] = entry("note", "note.txt", supportsMove = false)
        }, expected = StorageFailure.Unsupported::class.java)
        refused(provider = MoveProvider().apply {
            entries["archive" to "note.txt"] = entry("existing", "note.txt")
        }, expected = StorageFailure.Invalid::class.java)
        refused(to = SafMovePath("another-tree", listOf("Archive", "note.txt")), expected = StorageFailure.Unsupported::class.java)
        refused(to = movePath("Archive/renamed.txt"), expected = StorageFailure.Unsupported::class.java)
        refused(to = movePath("Archive/../note.txt"), expected = StorageFailure.Invalid::class.java)
        refused(provider = MoveProvider().apply {
            entries["root" to "Archive"] = entry("archive", "Archive")
        }, expected = StorageFailure.NotFound::class.java)
        refused(provider = MoveProvider().apply {
            entries["documents" to "note.txt"] = entry("note", "note.txt", isDir = true)
        }, to = movePath("Documents/note.txt/Child/note.txt"), expected = StorageFailure.Invalid::class.java)
        // Providers may expose one directory through more than one display path.
        refused(provider = MoveProvider().apply {
            entries["documents" to "note.txt"] = entry("note", "note.txt", isDir = true)
            entries["root" to "Alias"] = entry("note", "Alias", isDir = true)
        }, to = movePath("Alias/note.txt"), expected = StorageFailure.Invalid::class.java)
        refused(provider = MoveProvider().apply { returnedId = null }, expected = StorageFailure.Failed::class.java, dispatched = 1)
        refused(provider = MoveProvider().apply { confirmResult = false }, expected = StorageFailure.Failed::class.java, dispatched = 1)
    }

    private fun movePath(path: String) = SafMovePath("granted-tree", path.split('/'))

    private class MoveProvider : SafMoveOperations {
        override val rootId = "root"
        var granted = true
        var revokeBeforeMove = false
        var grantChecks = 0
        var returnedId: String? = "moved-note"
        var confirmResult = true
        val moves = mutableListOf<Triple<String, String, String>>()
        val entries = mutableMapOf(
            ("root" to "Documents") to entry("documents", "Documents", isDir = true),
            ("root" to "Archive") to entry("archive", "Archive", isDir = true),
            ("documents" to "note.txt") to entry("note", "note.txt"),
        )

        override fun hasReadWriteGrant(): Boolean {
            grantChecks++
            return granted && !(revokeBeforeMove && grantChecks > 1)
        }

        override fun child(parentId: String, name: String): SafMoveEntry? = entries[parentId to name]

        override fun move(documentId: String, sourceParentId: String, targetParentId: String): String? {
            moves += Triple(documentId, sourceParentId, targetParentId)
            val source = entries.entries.single { it.key.first == sourceParentId && it.value.id == documentId }
            entries.remove(source.key)
            if (confirmResult) {
                returnedId?.let { entries[targetParentId to source.value.name] = source.value.copy(id = it) }
            }
            return returnedId
        }
    }

    companion object {
        private fun entry(id: String, name: String, isDir: Boolean = false, supportsMove: Boolean = true) =
            SafMoveEntry(id, name, isDir, supportsMove)
    }

    @Test
    fun resolvesInsideRootAndBlocksPrivateAndTraversalPaths() {
        val temp = Files.createTempDirectory("nectarlink-storage-test").toFile()
        try {
            val root = File(temp, "shared").apply { mkdirs() }
            val outside = File(temp, "outside").apply { mkdirs() }
            val outsideSecret = File(outside, "secret.txt").apply { writeText("secret") }
            val androidData = File(root, "Android/data/app.nectarlink").apply { mkdirs() }
            File(androidData, "private.db").writeText("private")
            val docs = File(root, "Documents").apply { mkdirs() }
            val note = File(docs, "note.txt").apply { writeText("hello") }

            val blocked = listOf(
                File("/data"),
                File(root, "Android/data"),
                File(root, "Android/obb"),
            )

            val resolvedRoot = PhoneStorage.resolveExistingInRoot(root, blocked, "", allowRoot = true)
            assertEquals(root.canonicalFile, resolvedRoot)

            val resolvedNote = PhoneStorage.resolveExistingInRoot(
                root,
                blocked,
                "Documents/note.txt",
                allowRoot = false,
            )
            assertEquals(note.canonicalFile, resolvedNote)

            // Reject traversal & absolute paths
            for (bad in listOf("../outside/secret.txt", "/data/system", "Documents/../../outside/secret.txt")) {
                try {
                    PhoneStorage.resolveExistingInRoot(root, blocked, bad, allowRoot = false)
                    fail("expected Invalid for $bad")
                } catch (_: StorageFailure.Invalid) {
                }
            }

            // Reject blocked Android/data directory
            try {
                PhoneStorage.resolveExistingInRoot(
                    root,
                    blocked,
                    "Android/data/app.nectarlink/private.db",
                    allowRoot = false,
                )
                fail("expected Denied for Android/data")
            } catch (_: StorageFailure.Denied) {
            }

            try {
                PhoneStorage.resolveTargetInRoot(
                    root,
                    blocked,
                    "Android/data/app.nectarlink/new.db",
                )
                fail("expected Denied for target in Android/data")
            } catch (_: StorageFailure.Denied) {
            }

            // If the OS allows creating a symlink, verify symlink escape is rejected
            val linkFile = File(docs, "escape.txt")
            val symlinkCreated = runCatching {
                Files.createSymbolicLink(linkFile.toPath(), outsideSecret.toPath())
                true
            }.getOrDefault(false)
            if (symlinkCreated) {
                try {
                    PhoneStorage.resolveExistingInRoot(
                        root,
                        blocked,
                        "Documents/escape.txt",
                        allowRoot = false,
                    )
                    fail("expected Denied for symlink escaping root")
                } catch (_: StorageFailure.Denied) {
                }
            }
        } finally {
            temp.deleteRecursively()
        }
    }
}
