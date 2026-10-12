package ai.storyteller.wordcraft;

import com.google.androidgamesdk.GameActivity;

public final class MainActivity extends GameActivity {
    static {
        System.loadLibrary("wordcraft");
    }
}
