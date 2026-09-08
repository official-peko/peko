package com.example.harbor

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

class SettingsActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            val account by Session.account.collectAsState()
            Scaffold { padding ->
                Column(
                    Modifier.padding(padding).padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    Text("Settings")
                    account?.let {
                        Text("Signed in as ${it.displayName}")
                        Text(it.email)
                    }
                    TextButton(onClick = { open("https://harborapp.example/privacy") }) {
                        Text("Privacy policy")
                    }
                    TextButton(onClick = { open("https://harborapp.example/support") }) {
                        Text("Support")
                    }
                    TextButton(onClick = { open("https://harborapp.example/do-not-sell") }) {
                        Text("Do Not Sell or Share My Personal Information")
                    }
                    TextButton(onClick = {
                        Session.signOut()
                        finish()
                    }) {
                        Text("Sign out")
                    }
                }
            }
        }
    }

    private fun open(url: String) {
        startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
    }
}
