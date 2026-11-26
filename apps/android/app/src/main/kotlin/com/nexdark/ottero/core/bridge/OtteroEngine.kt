package com.nexdark.ottero.core.bridge

import android.content.Context
import androidx.compose.runtime.Stable
import com.nexdark.ottero.core.sync.SyncRealtimeClient
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import uniffi.ottero_mobile_ffi.AccountSessionBundle
import uniffi.ottero_mobile_ffi.AppInfo
import uniffi.ottero_mobile_ffi.CalendarEventItem
import uniffi.ottero_mobile_ffi.FocusSessionItem
import uniffi.ottero_mobile_ffi.HabitItem
import uniffi.ottero_mobile_ffi.HolidayItem
import uniffi.ottero_mobile_ffi.MobileAccountStatus
import uniffi.ottero_mobile_ffi.MobileSignInResult
import uniffi.ottero_mobile_ffi.MobileSyncReport
import uniffi.ottero_mobile_ffi.MobileTaskFilter
import uniffi.ottero_mobile_ffi.MobileVaultStatus
import uniffi.ottero_mobile_ffi.NoteDetail
import uniffi.ottero_mobile_ffi.NoteListItem
import uniffi.ottero_mobile_ffi.OtteroMobile
import uniffi.ottero_mobile_ffi.ProjectItem
import uniffi.ottero_mobile_ffi.SyncWsCredentials
import uniffi.ottero_mobile_ffi.TagCount
import uniffi.ottero_mobile_ffi.TaskItem
import uniffi.ottero_mobile_ffi.TodaySnapshot
import uniffi.ottero_mobile_ffi.TotpCodeSnapshot
import uniffi.ottero_mobile_ffi.UnifiedTrashSnapshot
import uniffi.ottero_mobile_ffi.VaultEntryDetail
import uniffi.ottero_mobile_ffi.VaultEntrySummary
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

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
    private val syncScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val syncMutex = Mutex()
    private val syncNeededAgain = AtomicBoolean(false)
    @Volatile
    private var autoSyncJob: Job? = null

    val realtime: SyncRealtimeClient = SyncRealtimeClient(this)
    val isRealtimeConnected: StateFlow<Boolean> = realtime.isLive

    private val _changes = MutableSharedFlow<Unit>(extraBufferCapacity = 32)
    val changes: SharedFlow<Unit> = _changes.asSharedFlow()

    /** Emits a local change signal and schedules a 500 ms debounced cloud sync if signed in. */
    private fun notifyMutation() {
        _changes.tryEmit(Unit)
        scheduleAutoSync()
    }

    private fun scheduleAutoSync() {
        autoSyncJob?.cancel()
        autoSyncJob = syncScope.launch {
            delay(500L)
            val signedIn = runCatching {
                withContext(dbDispatcher) { mobile.accountStatus().signedIn }
            }.getOrDefault(false)
            if (signedIn) {
                requestCoalescedSync()
            }
        }
    }

    /**
     * Runs a sync cycle, coalescing concurrent triggers (e.g. WebSocket
     * `seq_advanced` arriving mid-sync) into at most one follow-up pass.
     */
    suspend fun requestCoalescedSync() {
        if (!syncMutex.tryLock()) {
            syncNeededAgain.set(true)
            return
        }
        try {
            do {
                syncNeededAgain.set(false)
                val ran = runCatching {
                    withContext(Dispatchers.IO) {
                        mobile.syncNow().also { _changes.tryEmit(Unit) }
                    }
                }.isSuccess
            } while (ran && syncNeededAgain.getAndSet(false))
        } finally {
            syncMutex.unlock()
        }
    }

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
        mobile.createNote(title, body).also { notifyMutation() }
    }

    suspend fun updateNote(id: String, title: String, body: String): NoteDetail =
        withContext(dbDispatcher) {
            mobile.updateNote(id, title, body).also { notifyMutation() }
        }

    fun flushNoteAsync(id: String, title: String, body: String) {
        appScope.launch {
            runCatching {
                mobile.updateNote(id, title, body)
                notifyMutation()
            }
        }
    }

    suspend fun setNotePinned(id: String, pinned: Boolean): NoteDetail =
        withContext(dbDispatcher) {
            mobile.setNotePinned(id, pinned).also { notifyMutation() }
        }

    suspend fun trashNote(id: String) = withContext(dbDispatcher) {
        mobile.trashNote(id)
        notifyMutation()
    }

    suspend fun restoreNote(id: String): NoteDetail = withContext(dbDispatcher) {
        mobile.restoreNote(id).also { notifyMutation() }
    }

    suspend fun deleteNotePermanently(id: String) = withContext(dbDispatcher) {
        mobile.deleteNotePermanently(id)
        notifyMutation()
    }

    suspend fun listTasks(filter: MobileTaskFilter): List<TaskItem> = withContext(dbDispatcher) {
        mobile.listTasks(filter)
    }

    suspend fun quickAddTask(input: String, defaultToday: Boolean): TaskItem =
        withContext(dbDispatcher) {
            mobile.quickAddTask(input, defaultToday).also { notifyMutation() }
        }

    suspend fun updateTask(
        id: String,
        title: String,
        notes: String,
        priority: Int,
        dueDate: String?,
    ): TaskItem = withContext(dbDispatcher) {
        mobile.updateTask(id, title, notes, priority.toUByte(), dueDate).also {
            notifyMutation()
        }
    }

    suspend fun toggleTaskDone(id: String): TaskItem = withContext(dbDispatcher) {
        mobile.toggleTaskDone(id).also { notifyMutation() }
    }

    suspend fun trashTask(id: String) = withContext(dbDispatcher) {
        mobile.trashTask(id)
        notifyMutation()
    }

    suspend fun restoreTask(id: String): TaskItem = withContext(dbDispatcher) {
        mobile.restoreTask(id).also { notifyMutation() }
    }

    suspend fun listHabits(): List<HabitItem> = withContext(dbDispatcher) {
        mobile.listHabits()
    }

    suspend fun quickAddHabit(input: String): HabitItem = withContext(dbDispatcher) {
        mobile.quickAddHabit(input).also { notifyMutation() }
    }

    suspend fun tapHabitToday(id: String): HabitItem = withContext(dbDispatcher) {
        mobile.tapHabitToday(id).also { notifyMutation() }
    }

    suspend fun listEventsInRange(startIso: String, endIso: String): List<CalendarEventItem> =
        withContext(dbDispatcher) {
            mobile.listEventsInRange(startIso, endIso)
        }

    suspend fun quickAddEvent(input: String, defaultDateIso: String?): CalendarEventItem =
        withContext(dbDispatcher) {
            mobile.quickAddEvent(input, defaultDateIso).also { notifyMutation() }
        }

    suspend fun trashEvent(id: String) = withContext(dbDispatcher) {
        mobile.trashEvent(id)
        notifyMutation()
    }

    suspend fun restoreEvent(id: String) = withContext(dbDispatcher) {
        mobile.restoreEvent(id)
        notifyMutation()
    }

    suspend fun updateEvent(
        id: String,
        title: String,
        notes: String,
        location: String,
        dateIso: String,
        allDay: Boolean,
        startTime: String?,
        endTime: String?,
    ): CalendarEventItem = withContext(dbDispatcher) {
        mobile.updateEvent(id, title, notes, location, dateIso, allDay, startTime, endTime).also {
            notifyMutation()
        }
    }

    suspend fun listHolidaysInRange(
        startIso: String,
        endIso: String,
        countryCode: String? = null,
    ): List<HolidayItem> = withContext(dbDispatcher) {
        mobile.listHolidaysInRange(startIso, endIso, countryCode)
    }

    suspend fun unifiedTrashSnapshot(): UnifiedTrashSnapshot = withContext(dbDispatcher) {
        mobile.unifiedTrashSnapshot()
    }

    suspend fun deleteTaskPermanently(id: String) = withContext(dbDispatcher) {
        mobile.deleteTaskPermanently(id)
        notifyMutation()
    }

    suspend fun deleteEventPermanently(id: String) = withContext(dbDispatcher) {
        mobile.deleteEventPermanently(id)
        notifyMutation()
    }

    suspend fun emptyAllTrash(): UInt = withContext(dbDispatcher) {
        mobile.emptyAllTrash().also { notifyMutation() }
    }

    fun refreshLocal() {
        _changes.tryEmit(Unit)
    }

    suspend fun setTaskProject(id: String, projectId: String?): TaskItem =
        withContext(dbDispatcher) {
            mobile.setTaskProject(id, projectId).also { notifyMutation() }
        }

    suspend fun listProjects(): List<ProjectItem> = withContext(dbDispatcher) {
        mobile.listProjects()
    }

    suspend fun createProject(name: String): ProjectItem = withContext(dbDispatcher) {
        mobile.createProject(name).also { notifyMutation() }
    }

    suspend fun listSubtasks(parentId: String): List<TaskItem> = withContext(dbDispatcher) {
        mobile.listSubtasks(parentId)
    }

    suspend fun addSubtask(parentId: String, title: String): TaskItem =
        withContext(dbDispatcher) {
            mobile.addSubtask(parentId, title).also { notifyMutation() }
        }

    suspend fun updateHabit(
        id: String,
        name: String,
        scheduleCode: String,
        target: Int,
    ): HabitItem = withContext(dbDispatcher) {
        mobile.updateHabit(id, name, scheduleCode, target.coerceAtLeast(1).toUShort()).also {
            notifyMutation()
        }
    }

    suspend fun archiveHabit(id: String) = withContext(dbDispatcher) {
        mobile.archiveHabit(id)
        notifyMutation()
    }

    suspend fun deleteHabit(id: String) = withContext(dbDispatcher) {
        mobile.deleteHabit(id)
        notifyMutation()
    }

    suspend fun recordFocusSession(
        label: String,
        kindCode: String,
        plannedSecs: Int,
        actualSecs: Int,
        completed: Boolean,
    ): FocusSessionItem = withContext(dbDispatcher) {
        mobile.recordFocusSession(
            label,
            kindCode,
            plannedSecs.coerceAtLeast(0).toUInt(),
            actualSecs.coerceAtLeast(0).toUInt(),
            completed,
        ).also { notifyMutation() }
    }

    suspend fun listTodayFocusSessions(): List<FocusSessionItem> = withContext(dbDispatcher) {
        mobile.listTodayFocusSessions()
    }

    suspend fun vaultStatus(): MobileVaultStatus = withContext(dbDispatcher) {
        mobile.vaultStatus()
    }

    /** Argon2id derivation runs on [Dispatchers.Default] without blocking [dbDispatcher]. */
    suspend fun vaultSetup(masterPassword: String) = withContext(Dispatchers.Default) {
        mobile.vaultSetup(masterPassword)
        _changes.tryEmit(Unit)
    }

    /** Argon2id derivation runs on [Dispatchers.Default] without blocking [dbDispatcher]. */
    suspend fun vaultUnlock(masterPassword: String) = withContext(Dispatchers.Default) {
        mobile.vaultUnlock(masterPassword)
        _changes.tryEmit(Unit)
    }

    /** Immediate non-queued lock so backgrounding never waits behind SQLite or Argon2id. */
    fun lockVaultNow() {
        runCatching { mobile.vaultLock() }
        _changes.tryEmit(Unit)
    }

    suspend fun vaultLock() = withContext(Dispatchers.Default) {
        mobile.vaultLock()
        _changes.tryEmit(Unit)
    }

    suspend fun vaultListItems(query: String? = null): List<VaultEntrySummary> =
        withContext(dbDispatcher) {
            mobile.vaultListItems(query)
        }

    suspend fun vaultGetItem(id: String): VaultEntryDetail = withContext(dbDispatcher) {
        mobile.vaultGetItem(id)
    }

    suspend fun vaultSaveLogin(
        id: String?,
        title: String,
        username: String,
        password: String,
        url: String,
        totpSecret: String,
        notes: String,
    ): VaultEntryDetail = withContext(dbDispatcher) {
        mobile.vaultSaveLogin(id, title, username, password, url, totpSecret, notes).also {
            notifyMutation()
        }
    }

    suspend fun vaultTrashItem(id: String) = withContext(dbDispatcher) {
        mobile.vaultTrashItem(id)
        notifyMutation()
    }

    suspend fun vaultRestoreItem(id: String) = withContext(dbDispatcher) {
        mobile.vaultRestoreItem(id)
        notifyMutation()
    }

    suspend fun vaultGeneratePassword(length: Int = 20): String = withContext(Dispatchers.Default) {
        mobile.vaultGeneratePassword(length.toUByte())
    }

    suspend fun vaultTotpNow(secret: String): TotpCodeSnapshot = withContext(Dispatchers.Default) {
        mobile.vaultTotpNow(secret)
    }

    suspend fun vaultSaveCard(
        id: String?,
        title: String,
        cardholder: String,
        number: String,
        expiry: String,
        cvv: String,
        notes: String,
    ): VaultEntryDetail = withContext(dbDispatcher) {
        mobile.vaultSaveCard(id, title, cardholder, number, expiry, cvv, notes).also {
            notifyMutation()
        }
    }

    suspend fun vaultSaveSecureNote(
        id: String?,
        title: String,
        notes: String,
    ): VaultEntryDetail = withContext(dbDispatcher) {
        mobile.vaultSaveSecureNote(id, title, notes).also { notifyMutation() }
    }

    suspend fun vaultToggleFavorite(id: String): Boolean = withContext(dbDispatcher) {
        mobile.vaultToggleFavorite(id).also { notifyMutation() }
    }

    suspend fun vaultAutofillMatches(hint: String): List<VaultEntryDetail> =
        withContext(dbDispatcher) {
            mobile.vaultAutofillMatches(hint)
        }

    suspend fun accountStatus(): MobileAccountStatus = withContext(dbDispatcher) {
        mobile.accountStatus()
    }

    suspend fun accountSignIn(
        email: String,
        password: String,
        secretKey: String,
    ): MobileSignInResult = withContext(Dispatchers.IO) {
        mobile.accountSignIn(email, password, secretKey).also {
            _changes.tryEmit(Unit)
            if (it.session != null) realtime.reconnectNow()
        }
    }

    suspend fun accountVerifyTwoStep(
        code: String,
        isBackupCode: Boolean,
    ): MobileSignInResult = withContext(Dispatchers.IO) {
        mobile.accountVerifyTwoStep(code, isBackupCode).also {
            _changes.tryEmit(Unit)
            if (it.session != null) realtime.reconnectNow()
        }
    }

    suspend fun accountRestoreSession(session: AccountSessionBundle) = withContext(Dispatchers.IO) {
        mobile.accountRestoreSession(
            session.email,
            session.accountId,
            session.bearerToken,
            session.accountKeyB64,
        )
        _changes.tryEmit(Unit)
        realtime.reconnectNow()
    }

    suspend fun accountSignOut() = withContext(Dispatchers.IO) {
        realtime.stop()
        mobile.accountSignOut()
        _changes.tryEmit(Unit)
    }

    suspend fun syncWsCredentials(): SyncWsCredentials? = withContext(dbDispatcher) {
        mobile.syncWsCredentials()
    }

    suspend fun vaultUnlockWithAccount(): Boolean = withContext(dbDispatcher) {
        mobile.vaultUnlockWithAccount().also { unlocked ->
            if (unlocked) _changes.tryEmit(Unit)
        }
    }

    suspend fun syncNow(): MobileSyncReport = withContext(Dispatchers.IO) {
        syncMutex.withLock {
            mobile.syncNow().also { _changes.tryEmit(Unit) }
        }
    }

    companion object {
        @Volatile
        private var instance: OtteroEngine? = null
        private val initMutex = Mutex()

        fun currentOrNull(): OtteroEngine? = instance

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
