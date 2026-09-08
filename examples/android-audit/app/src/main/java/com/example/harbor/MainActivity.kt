package com.example.harbor

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

class MainActivity : ComponentActivity() {
    private val session = Session

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Analytics.start(applicationContext)

        setContent {
            val account by session.account.collectAsState()
            Scaffold { padding ->
                Column(Modifier.padding(padding).padding(16.dp)) {
                    if (account == null) {
                        SignInScreen()
                    } else {
                        NoteListScreen(
                            onSettings = {
                                startActivity(Intent(this@MainActivity, SettingsActivity::class.java))
                            }
                        )
                    }
                }
            }
        }
    }
}
