// The native iPad app's player: YouTube's IFrame player behind a small message bridge.
// Calls in: window.cubby. Messages out: webkit.messageHandlers.cubby (absent in a browser).
(function () {
	var player = null;
	var timer = null;
	var apiReady = false;
	var pending = null;

	function post(msg) {
		try {
			var h = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.cubby;
			if (h) h.postMessage(msg);
		} catch (e) {}
	}

	function stopTicks() {
		if (timer !== null) clearInterval(timer);
		timer = null;
	}

	function tick() {
		if (!player) return;
		post({ type: 'time', t: player.getCurrentTime(), d: player.getDuration() });
	}

	function onStateChange(e) {
		post({ type: 'state', state: e.data });
		var id = null;
		try {
			id = player.getVideoData().video_id;
		} catch (err) {}
		post({ type: 'video', id: id });
		if (e.data === 1) {
			if (timer === null) timer = setInterval(tick, 500);
		} else {
			stopTicks();
		}
	}

	function create(videoId, start) {
		player = new window.YT.Player('p', {
			host: 'https://www.youtube-nocookie.com',
			videoId: videoId,
			playerVars: {
				autoplay: 1,
				rel: 0,
				playsinline: 1,
				fs: 0,
				iv_load_policy: 3,
				controls: 0,
				disablekb: 1,
				start: Math.floor(start || 0),
				origin: location.origin
			},
			events: {
				onReady: function () {
					post({ type: 'ready' });
				},
				onStateChange: onStateChange,
				onError: function (e) {
					post({ type: 'error', code: e.data });
				},
				onAutoplayBlocked: function () {
					post({ type: 'autoplayBlocked' });
				}
			}
		});
	}

	window.onYouTubeIframeAPIReady = function () {
		apiReady = true;
		if (pending) {
			var p = pending;
			pending = null;
			create(p.videoId, p.start);
		}
	};

	window.cubby = {
		load: function (videoId, start) {
			window.cubby.destroy();
			if (apiReady) create(videoId, start);
			else pending = { videoId: videoId, start: start };
		},
		play: function () {
			if (player) player.playVideo();
		},
		pause: function () {
			if (player) player.pauseVideo();
		},
		seek: function (t) {
			if (player) player.seekTo(t, true);
		},
		destroy: function () {
			stopTicks();
			pending = null;
			if (player) {
				try {
					player.destroy();
				} catch (e) {}
				player = null;
			}
			if (!document.getElementById('p')) {
				var d = document.createElement('div');
				d.id = 'p';
				document.body.insertBefore(d, document.body.firstChild);
			}
		}
	};
})();
