package com.nexdark.ottero.core.bridge

import android.content.Context
import androidx.compose.runtime.Stable
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import uniffi.ottero_mobile_ffi.AppInfo
import uniffi.ottero_mobile_ffi.CalendarEventItem
import uniffi.ottero_mobile_ffi.HabitItem
import uniffi.ottero_mobile_ffi.MobileTaskFilter
import uniffi.ottero_mobile_ffi.NoteDetail
import uniffi.ottero_mobile_ffi.NoteListItem
import uniffi.ottero_mobile_ffi.OtteroMobile
import uniffi.ottero_mobile_ffi.TagCount
import uniffi.ottero_mobile_ffi.TaskItem
import uniffi.ottero_mobile_ffi.TodaySnapshot
import java.io.File

/**
 * Thread-safe Kotlin facade over Ottero's Rust SQLite core (`OtteroMobile`).
 * Uses a single serialized IO dispatcher to eliminate mutex contention and
 * preserve strict FIFO ordering of reads and writes off the 120Hz main thread.
 */
@OptIn(ExperimentalCoroutinesApi::class)
@Stable
class OtteroEngine(private val mobile: OtteroMobile) {

    private val dbDispatcher = Dispatchers.IO.limitedParallelism(1)
    private val appScope = CoroutineScope(SupervisorJob() + dbDispatcher)

    private val _changes = MutableSharedFlow<Unit>(extraBufferCapacity = 32)
    val changes: SharedFlow<Unit> = _changes.asSharedFlow()

    suspend fun appInfo(): AppInfo = withContext(dbDispatcher) {
        mobile.appInfo()
    }

    suspend fun todaySnapshot(): TodaySnapshot = withContext(dbDispatcher) {
        mobile.todaySnapshot()
    }

    suspend fun listNotes(query: String? = null): List<NoteListItem> = withContext(dbDispatcher) {
        mobile.listNotes(query)
    }

    suspend fun listTrashedNotes(): List<NoteListItem> = withContext(dbDispatcher) {
        mobile.listTrashedNotes()
    }

    suspend fun listTags(): List<TagCount> = withContext(dbDispatcher) {
        mobile.listTags()
    }

    suspend fun listNotesByTag(tag: String): List<NoteListItem> = withContext(dbDispatcher) {
        mobile.listNotesByTag(tag)
    }

    suspend fun getNote(id: String): NoteDetail = withContext(dbDispatcher) {
        mobile.getNote(id)
    }

    suspend fun createNote(title: String, body: String): NoteDetail = withContext(dbDispatcher) {
        mobile.createNote(title, body).also { _changes.tryEmit(Unit) }
    }

    suspend fun updateNote(id: String, title: String, body: String): NoteDetail =
        withContext(dbDispatcher) {
            mobile.updateNote(id, title, body).also { _changes.tryEmit(Unit) }
        }

    fun flushNoteAsync(id: String, title: String, body: String) {
        appScope.launch {
            runCatching {
                mobile.updateNote(id, title, body)
                _changes.tryEmit(Unit)
            }
        }
    }

    suspend fun setNotePinned(id: String, pinned: Boolean): NoteDetail =
        withContext(dbDispatcher) {
            mobile.setNotePinned(id, pinned).also { _changes.tryEmit(Unit) }
        }

    suspend fun trashNote(id: String) = withContext(dbDispatcher) {
        mobile.trashNote(id)
        _changes.tryEmit(Unit)
    }

    suspend fun restoreNote(id: String): NoteDetail = withContext(dbDispatcher) {
        mobile.restoreNote(id).also { _changes.tryEmit(Unit) }
    }

    suspend fun deleteNotePermanently(id: String) = withContext(dbDispatcher) {
        mobile.deleteNotePermanently(id)
        _changes.tryEmit(Unit)
    }

    suspend fun listTasks(filter: MobileTaskFilter): List<TaskItem> = withContext(dbDispatcher) {
        mobile.listTasks(filter)
    }

    suspend fun quickAddTask(input: String, defaultToday: Boolean): TaskItem =
        withContext(dbDispatcher) {
            mobile.quickAddTask(input, defaultToday).also { _changes.tryEmit(Unit) }
        }

    suspend fun updateTask(
        id: String,
        title: String,
        notes: String,
        priority: Int,
        dueDate: String?,
    ): TaskItem = withContext(dbDispatcher) {
        mobile.updateTask(id, title, notes, priority.toUByte(), dueDate).also {
            _changes.tryEmit(Unit)
        }
    }

    suspend fun toggleTaskDone(id: String): TaskItem = withContext(dbDispatcher) {
        mobile.toggleTaskDone(id).also { _changes.tryEmit(Unit) }
    }

    suspend fun trashTask(id: String) = withContext(dbDispatcher) {
        mobile.trashTask(id)
        _changes.tryEmit(Unit)
    }

    suspend fun restoreTask(id: String): TaskItem = withContext(dbDispatcher) {
        mobile.restoreTask(id).also { _changes.tryEmit(Unit) }
    }

    suspend fun listHabits(): List<HabitItem> = withContext(dbDispatcher) {
        mobile.listHabits()
    }

    suspend fun quickAddHabit(input: String): HabitItem = withContext(dbDispatcher) {
        mobile.quickAddHabit(input).also { _changes.tryEmit(Unit) }
    }

    suspend fun tapHabitToday(id: String): HabitItem = withContext(dbDispatcher) {
        mobile.tapHabitToday(id).also { _changes.tryEmit(Unit) }
    }

    suspend fun listEventsInRange(startIso: String, endIso: String): List<CalendarEventItem> =
        withContext(dbDispatcher) {
            mobile.listEventsInRange(startIso, endIso)
        }

    suspend fun quickAddEvent(input: String, defaultDateIso: String?): CalendarEventItem =
        withContext(dbDispatcher) {
            mobile.quickAddEvent(input, defaultDateIso).also { _changes.tryEmit(Unit) }
        }

    suspend fun trashEvent(id: String) = withContext(dbDispatcher) {
        mobile.trashEvent(id)
        _changes.tryEmit(Unit)
    }

    suspend fun restoreEvent(id: String) = withContext(dbDispatcher) {
        mobile.restoreEvent(id)
        _changes.tryEmit(Unit)
    }

    companion object {
        @Volatile
        private var instance: OtteroEngine? = null
        private val initMutex = Mutex()

        suspend fun open(context: Context): Result<OtteroEngine> {
            instance?.let { return Result.success(it) }
            val appContext = context.applicationContext
            return withContext(Dispatchers.IO) {
                initMutex.withLock {
                    instance?.let { return@withLock Result.success(it) }
                    runCatching {
                        val dbFile = File(appContext.filesDir, "ottero.sqlite3")
                        OtteroEngine(OtteroMobile.open(dbFile.absolutePath)).also { instance = it }
                    }
                }
            }
        }
    }
}
