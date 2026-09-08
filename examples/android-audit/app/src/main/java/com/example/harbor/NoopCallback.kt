package com.example.harbor

import okhttp3.Call
import okhttp3.Callback
import okhttp3.Response
import java.io.IOException

/** Metrics are best effort. A failed send is dropped rather than retried. */
object NoopCallback : Callback {
    override fun onFailure(call: Call, e: IOException) = Unit

    override fun onResponse(call: Call, response: Response) {
        response.close()
    }
}
