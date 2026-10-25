<script lang="ts">
	import Child from '$lib/Child.svelte';
</script>

<!-- One child shell for every child URL: the route decides the view, the shell keeps the player alive across in-app navigation. -->
<Child />
