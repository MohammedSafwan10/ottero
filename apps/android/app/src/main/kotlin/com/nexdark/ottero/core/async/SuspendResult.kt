package com.nexdark.ottero.core.async

import kotlinx.coroutines.CancellationException

/**
 * Like [kotlin.runCatching], but ALWAYS rethrows [CancellationException] so
 * structured concurrency cancellations (e.g. fast tab switches leaving
 * composition) are never misreported as user-facing errors.
 */
inline fun <T> runSuspendCatching(block: () -> T): Result<T> = try {
    Result.success(block())
} catch (ce: CancellationException) {
    throw ce
} catch (t: Throwable) {
    Result.failure(t)
}
