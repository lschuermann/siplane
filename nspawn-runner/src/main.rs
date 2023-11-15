#[tokio::main]
async fn main() {
    use futures::{TryStreamExt};
    use eventsource_client::{Client, SSE};

    let mut client = eventsource_client::ClientBuilder::for_url("http://localhost:4000/api/runner/v0/boards/8bfde627-583b-4935-a3e7-f259868eaec2/sse").unwrap()
	// .header("Authorization", "Basic username:password")?
	.build();

    let mut stream = Box::pin(client.stream())
	.map_ok(|event| match event {
            SSE::Comment(comment) => println!("got a comment event: {:?}", comment),
            SSE::Event(evt) => println!("got an event: {}", evt.event_type)
	})
	.map_err(|e| println!("error streaming events: {:?}", e));

    while let Ok(Some(_)) = stream.try_next().await {
	// Do nothing
    }
}
