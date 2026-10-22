package dev.nimadir.shell;

import android.app.Activity;
import android.view.ViewGroup;
import android.webkit.WebChromeClient;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.FrameLayout;

/**
 * A web page over the shell's page area (ADR-6): the platform's WebView,
 * which is a Java view and lives on the UI thread. The native shell calls
 * these from its own thread; each does its work on the UI thread and does
 * not wait for it.
 *
 * Module 1, quarantined here as webview.rs quarantines it on the desktop:
 * nothing of the shell is bridged to the page.
 */
public final class WebPane {
    private static WebView view;
    private static String loaded;
    private static volatile boolean canGoBack;

    private WebPane() {}

    /** Show url over x, y, w, h: physical pixels from the window's corner. */
    public static void show(final Activity activity, final String url,
                            final int x, final int y, final int w, final int h) {
        activity.runOnUiThread(() -> {
            if (view == null) {
                view = new WebView(activity);
                view.getSettings().setJavaScriptEnabled(true);
                view.getSettings().setDomStorageEnabled(true);
                view.setWebViewClient(new WebViewClient() {
                    @Override
                    public void doUpdateVisitedHistory(WebView v, String u, boolean reload) {
                        canGoBack = v.canGoBack();
                    }
                });
                view.setWebChromeClient(new WebChromeClient());
                activity.addContentView(view, place(new FrameLayout.LayoutParams(w, h), x, y));
            } else {
                ViewGroup.LayoutParams p = view.getLayoutParams();
                if (p instanceof FrameLayout.LayoutParams) {
                    FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) p;
                    if (lp.leftMargin != x || lp.topMargin != y || lp.width != w || lp.height != h) {
                        lp.width = w;
                        lp.height = h;
                        view.setLayoutParams(place(lp, x, y));
                    }
                }
            }
            if (!url.equals(loaded)) {
                loaded = url;
                canGoBack = false;
                view.loadUrl(url);
            }
        });
    }

    /** Remove the page and destroy its WebView: scripts, timers and all. */
    public static void hide(final Activity activity) {
        activity.runOnUiThread(() -> {
            if (view == null) {
                return;
            }
            ViewGroup parent = (ViewGroup) view.getParent();
            if (parent != null) {
                parent.removeView(view);
            }
            view.destroy();
            view = null;
            loaded = null;
            canGoBack = false;
        });
    }

    /** Whether the page has somewhere to go back to, as of its last change. */
    public static boolean canGoBack() {
        return canGoBack;
    }

    /** Back within the page. */
    public static void goBack(final Activity activity) {
        activity.runOnUiThread(() -> {
            if (view != null && view.canGoBack()) {
                view.goBack();
            }
        });
    }

    private static FrameLayout.LayoutParams place(FrameLayout.LayoutParams lp, int x, int y) {
        lp.leftMargin = x;
        lp.topMargin = y;
        return lp;
    }
}
