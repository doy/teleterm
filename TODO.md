these are things that i know need work (and i would welcome patches for), but
i'm also interested in anything else you think would make this software more
useful.

* packages for more operating systems
    * homebrew is probably a big one?
* grpc
    * although the existing hand-rolled protocol works fine, it does make it
      annoying to deal with things like sending the traffic through http
      proxies. pretty much the only reason this isn't using grpc is because i
      couldn't get any of the existing grpc crates to work reasonably, but
      that's likely more my fault than the fault of the crates themselves.
    * grpc would also make it much easier to enable compression (ideally we
      could just enable http compression at the library level), which is
      something that would be enormously helpful here (most terminal output is
      quite compressible)
* block (server-side) and hide (client-side) functionality
    * it's pretty important for any social software
* integration tests
    * for instance, spin up separate server, stream, and watch subprocesses,
      and write tests for their stdout
    * key_reader could also use some tests, although it's a bit tricky - i
      basically want to be able to write a binary that uses key_reader, and
      have the test spawn that binary as a subprocess and write tests against
      that, but i'm not sure how to make cargo do that
* watch ui improvements
    * should be able to sort by more things than just idle time
    * color more things (idle time colors might be useful, especially if
      support is added for different sorting methods)
    * get extended metadata about a stream somehow (maybe with shift+letter or
      something?) - things like names of watchers, etc
* different authentication methods
    * adding new oauth providers should be pretty trivial
    * mtls/client cert auth would also be pretty useful
* some kind of indication during streaming for when a watcher connects
    * visual bell should be pretty easy, but we might be able to be fancier by
      drawing a popup on the terminal somewhere, and just refreshing the screen
      from the buffer after some timeout
    * if we go the popup drawing route, it could also potentially be used for
      error message displays
* ability to adjust `tt play` playback speed
