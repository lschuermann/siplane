defmodule SiplaneWeb.API.Runner.V0.BoardController do
  use SiplaneWeb, :controller
  require Logger

  def index(conn, _params) do
    # The home page is often custom made,
    # so skip the default app layout.
    send_resp(conn, :ok, "hello!")
  end

  def sse_conn(conn, %{"id" => board_id_str}) do
    case UUID.info(board_id_str) do
      {:error, _} ->
	conn
	|> put_status(:bad_request)
	|> put_view(SiplaneWeb.API.ErrorView)
	|> render("bad_request.json",
	  description: "Provided board ID is not a valid UUID."
	)

      {:ok, parsed_board_id} ->
	# TODO: check that this board actually exist and validate the
	# token provided by the runner...
	{:ok, orch_pid} = Keyword.get(parsed_board_id, :binary)
  	|> Siplane.BoardOrchestrator.get_or_start()

	# TODO: validate the auth connection

	conn = conn
	|> put_resp_header("Cache-Control", "no-cache")
	|> put_resp_header("Connection", "keep-alive")
	|> put_resp_header("Content-Type", "text/event-stream; charset=utf-8")
	|> send_chunked(200)

	# Register this request handler process as a connection for
	# this runner to the orchestrator:
	{:ok, last_will_and_testament} =
	  Siplane.BoardOrchestrator.connect_runner orch_pid

	# Receive runner messages and forward them to SSE in a loop:
	sse_loop conn, orch_pid, last_will_and_testament
    end
  end

  defp sse_loop(conn, orch_pid, last_will_and_testament) do
    cont = receive do
      {:runner_conn, {^orch_pid, _}, :msg, msg} ->
	# Send message:
	chunk(conn, "event: message\ndata: #{Jason.encode! msg}\n\n")
	true

      {:DOWN, _ref, :process, ^orch_pid, _type} ->
	# When the orchestrator goes down, we should close SSE
	chunk(conn, "event: close\ndata: #{Jason.encode! last_will_and_testament}\n\n")
	false

      other ->
	# Ignore any unrelated messages:
	true
    end

    if cont do
      sse_loop conn, orch_pid, last_will_and_testament
    else
      conn
    end
  end

end
