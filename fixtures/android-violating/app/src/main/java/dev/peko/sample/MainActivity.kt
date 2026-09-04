package dev.peko.bad

import android.os.Bundle
import androidx.appcompat.app.AppCompatActivity
import com.google.android.gms.ads.identifier.AdvertisingIdClient

class MainActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val info = AdvertisingIdClient.getAdvertisingIdInfo(this)
        android.util.Log.d("bad", info.id.orEmpty())
    }
}
