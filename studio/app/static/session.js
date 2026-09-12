// The token arrives only in the URL fragment and becomes an HttpOnly cookie.
window.studioSessionReady = (async () => {
  const fragment = new URLSearchParams(location.hash.slice(1));
  const token = fragment.get('session');
  if (!token) return;
  history.replaceState(null, '', location.pathname);
  const response = await fetch('/session', { method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify({token}) });
  if (!response.ok) throw new Error('Abra novamente o endereço informado pelo Studio.');
})();
