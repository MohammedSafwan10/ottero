package com.nexdark.ottero.core.sync

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log
import com.nexdark.ottero.core.bridge.OtteroEngine
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/**
 * Foreground-only WebSocket listener for Ottero Cloud sync (`/v1/sync/ws`).
 *
 * Matches Desktop's `apps/desktop/src/sync/ws.rs` protocol:
 * - Authenticates with `Authorization: Bearer <token>` and disables redirects
 *   so the bearer token can never be forwarded to another host or downgraded.
 * - Triggers an immediate coalesced sync when `seq_advanced` arrives.
 * - Automatically reconnects with exponential backoff (2s..30s) and on
 *   Android network recovery via [ConnectivityManager.NetworkCallback].
 * - Pauses cleanly in the background (`ON_STOP`) so battery impact is zero.
 */
class SyncRealtimeClient(
    private val engine: OtteroEngine,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val httpClient: OkHttpClient = OkHttpClient.Builder()
        .followRedirects(false)
        .followSslRedirects(false)
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .pingInterval(25, TimeUnit.SECONDS)
        .build()

    private val _isLive = MutableStateFlow(false)
    val isLive: StateFlow<Boolean> = _isLive.asStateFlow()

    private val running = AtomicBoolean(false)
    private val terminalStop = AtomicBoolean(false)
    private val generation = AtomicLong(0L)

    @Volatile
    private var activeSocket: WebSocket? = null
    @Volatile
    private var appContext: Context? = null
    @Volatile
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    @Volatile
    private var reconnectJob: Job? = null
    @Volatile
    private var pingJob: Job? = null
    @Volatile
    private var fallbackPollJob: Job? = null
    @Volatile
    private var backoffMs: Long = INITIAL_BACKOFF_MS

    fun start(context: Context) {
        appContext = context.applicationContext
        terminalStop.set(false)
        if (!running.compareAndSet(false, true)) {
            if (activeSocket == null && !_isLive.value) {
                connect(resetBackoff = true)
            }
            return
        }
        registerNetworkCallback(context.applicationContext)
        startFallbackPoller()
        connect(resetBackoff = true)
    }

    fun reconnectNow() {
        terminalStop.set(false)
        if (!running.get()) return
        connect(resetBackoff = true)
    }

    fun stop() {
        running.set(false)
        generation.incrementAndGet()
        reconnectJob?.cancel()
        reconnectJob = null
        pingJob?.cancel()
        pingJob = null
        fallbackPollJob?.cancel()
        fallbackPollJob = null
        unregisterNetworkCallback()
        closeSocket()
        _isLive.value = false
    }

    private fun closeSocket() {
        val sock = activeSocket
        activeSocket = null
        runCatching { sock?.close(1000, "background") }
    }

    private fun startFallbackPoller() {
        fallbackPollJob?.cancel()
        fallbackPollJob = scope.launch {
            while (isActive && running.get()) {
                delay(FALLBACK_POLL_MS)
                if (!running.get()) break
                val creds = runCatching { engine.syncWsCredentials() }.getOrNull()
                if (creds != null) {
                    if (activeSocket == null && !terminalStop.get()) {
                        connect(resetBackoff = false)
                    }
                    engine.requestCoalescedSync()
                }
            }
        }
    }

    private fun connect(resetBackoff: Boolean) {
        if (!running.get() || terminalStop.get()) return
        if (resetBackoff) backoffMs = INITIAL_BACKOFF_MS

        val gen = generation.incrementAndGet()
        reconnectJob?.cancel()
        pingJob?.cancel()
        closeSocket()
        _isLive.value = false

        scope.launch {
            if (!running.get() || generation.get() != gen) return@launch
            val creds = runCatching { engine.syncWsCredentials() }.getOrNull() ?: return@launch
            if (!creds.wsUrl.startsWith("wss://")) return@launch

            val request = Request.Builder()
                .url(creds.wsUrl)
                .header("Authorization", "Bearer ${creds.bearerToken}")
                .build()

            val socket = httpClient.newWebSocket(
                request,
                object : WebSocketListener() {
                    override fun onOpen(webSocket: WebSocket, response: Response) {
                        if (generation.get() != gen || !running.get()) {
                            webSocket.close(1000, "stale")
                            return
                        }
                        Log.d(TAG, "WS connected (status=${response.code})")
                        backoffMs = INITIAL_BACKOFF_MS
                        _isLive.value = true
                        startAppPing(webSocket, gen)
                    }

                    override fun onMessage(webSocket: WebSocket, text: String) {
                        if (generation.get() != gen || !running.get()) return
                        handleServerFrame(text)
                    }

                    override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                        webSocket.close(1000, null)
                    }

                    override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                        if (generation.get() != gen) return
                        Log.d(TAG, "WS closed (code=$code)")
                        _isLive.value = false
                        pingJob?.cancel()
                        activeSocket = null
                        scheduleReconnect(gen)
                    }

                    override fun onFailure(
                        webSocket: WebSocket,
                        t: Throwable,
                        response: Response?,
                    ) {
                        if (generation.get() != gen) return
                        Log.w(TAG, "WS failure (http=${response?.code}, err=${t.javaClass.simpleName})")
                        _isLive.value = false
                        pingJob?.cancel()
                        activeSocket = null
                        val code = response?.code
                        if (code == 401 || code == 403 || code == 402) {
                            terminalStop.set(true)
                            return
                        }
                        scheduleReconnect(gen)
                    }
                },
            )
            activeSocket = socket
        }
    }

    private fun startAppPing(webSocket: WebSocket, gen: Long) {
        pingJob?.cancel()
        pingJob = scope.launch {
            while (isActive && running.get() && generation.get() == gen) {
                delay(PING_INTERVAL_MS)
                if (!running.get() || generation.get() != gen) break
                val sent = runCatching { webSocket.send("""{"type":"ping"}""") }.getOrDefault(false)
                if (!sent) break
            }
        }
    }

    private fun handleServerFrame(text: String) {
        val json = runCatching { JSONObject(text) }.getOrNull() ?: return
        when (json.optString("type")) {
            "seq_advanced" -> {
                val latestSeq = json.optLong("latest_seq", -1L)
                Log.d(TAG, "WS seq_advanced latest_seq=$latestSeq")
                scope.launch {
                    val creds = runCatching { engine.syncWsCredentials() }.getOrNull()
                    val lastPull = creds?.lastPullSeq ?: 0L
                    if (latestSeq < 0L || latestSeq > lastPull) {
                        engine.requestCoalescedSync()
                    }
                }
            }
            "device_revoked" -> {
                terminalStop.set(true)
                closeSocket()
                _isLive.value = false
            }
            "entitlement_changed" -> {
                if (!json.optBoolean("active", true)) {
                    terminalStop.set(true)
                    closeSocket()
                    _isLive.value = false
                }
            }
            "pong" -> Unit
        }
    }

    private fun scheduleReconnect(expectedGen: Long) {
        if (!running.get() || terminalStop.get() || generation.get() != expectedGen) return
        val delayNow = backoffMs
        backoffMs = (backoffMs * 2L).coerceAtMost(MAX_BACKOFF_MS)
        reconnectJob?.cancel()
        reconnectJob = scope.launch {
            delay(delayNow)
            if (running.get() && !terminalStop.get() && generation.get() == expectedGen) {
                connect(resetBackoff = false)
            }
        }
    }

    private fun registerNetworkCallback(context: Context) {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            ?: return
        unregisterNetworkCallback()
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                if (!running.get() || terminalStop.get()) return
                if (!_isLive.value) {
                    connect(resetBackoff = true)
                }
                scope.launch { engine.requestCoalescedSync() }
            }
        }
        runCatching {
            cm.registerNetworkCallback(request, cb)
            networkCallback = cb
        }
    }

    private fun unregisterNetworkCallback() {
        val cb = networkCallback ?: return
        networkCallback = null
        val cm = appContext?.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            ?: return
        runCatching { cm.unregisterNetworkCallback(cb) }
    }

    private companion object {
        const val TAG = "OtteroSync"
        const val INITIAL_BACKOFF_MS = 2_000L
        const val MAX_BACKOFF_MS = 30_000L
        const val PING_INTERVAL_MS = 25_000L
        const val FALLBACK_POLL_MS = 60_000L
    }
}
