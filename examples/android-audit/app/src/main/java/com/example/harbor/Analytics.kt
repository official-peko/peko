package com.example.harbor

import android.content.Context
import android.util.Log
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.util.UUID

/** Events go to our own metrics service and to the attribution partner. */
object Analytics {
    private const val TAG = "Harbor"
    private const val ENDPOINT = "https://metrics.harborapp.example/v1/events"
    private val client = OkHttpClient()
    private var installId = ""

    fun start(context: Context) {
        val prefs = context.getSharedPreferences("harbor", Context.MODE_PRIVATE)
        installId = prefs.getString("install_id", null) ?: UUID.randomUUID().toString().also {
            prefs.edit().putString("install_id", it).apply()
        }
        track("launch")
    }

    fun identify(account: Account) {
        Log.d(TAG, "identify ${account.email} token=${account.token}")
        send(
            JSONObject()
                .put("type", "identify")
                .put("user_id", account.id)
                .put("email", account.email)
                .put("name", account.displayName)
                .put("install_id", installId)
        )
    }

    fun track(event: String) {
        send(
            JSONObject()
                .put("type", "track")
                .put("event", event)
                .put("install_id", installId)
        )
    }

    private fun send(body: JSONObject) {
        val request = Request.Builder()
            .url(ENDPOINT)
            .post(body.toString().toRequestBody("application/json".toMediaType()))
            .build()
        client.newCall(request).enqueue(NoopCallback)
    }
}
