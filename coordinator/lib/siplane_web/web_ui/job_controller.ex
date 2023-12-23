defmodule SiplaneWeb.WebUI.JobController do
  use SiplaneWeb, :controller
  import Ecto.Query

  plug :put_view, html: SiplaneWeb.WebUI.PageHTML

  def render(assigns) do
    SiplaneWeb.WebUI.PageHTML.job(assigns)
  end

  # TODO!
end

defmodule SiplaneWeb.WebUI.JobController.Live do
  use SiplaneWeb, :live_view
  import Ecto.Query

  defp update_job_state(socket) do
    ecto_job_id = Ecto.UUID.load!(socket.assigns.job_id)

    # Query job information:
    socket =
      socket
      |> assign(job: (
	  Siplane.Repo.one!(
	    from j in Siplane.Job, where: j.id == ^ecto_job_id
	  )
	  |> Siplane.Repo.preload(:board)
	))
  end

  @impl true
  def render(assigns) do
    SiplaneWeb.WebUI.PageHTML.job(assigns)
  end

  @impl true
  def handle_params(_params, uri, socket) do
    {:noreply, assign(socket, current_uri: uri)}
  end

  @impl true
  def mount(%{"id" => job_id_str} = _params, _session, socket) do
    case UUID.info(job_id_str) do
      {:error, _} ->
	{:error, :uuid_invalid}

      {:ok, parsed_job_id} ->
	job_id = Keyword.get(parsed_job_id, :binary)
	socket = assign(socket, job_id: job_id)

	socket = update_job_state(socket)

	# Subscribe to log messages
	:ok = Siplane.Job.subscribe job_id

	{
	  :ok,
	  assign(
	    socket,
	    log_events: Siplane.Job.job_log(job_id),
	    # Fetch initial console log
	    console_log: "",
	  ),
	}
    end
  end

  @impl true
  def handle_info({:job_event, _job_id, :log_event, event}, socket) do
    {
      :noreply,
      assign(
	socket,
	log_events: [ event | socket.assigns.log_events ]
      )
    }
  end

  @impl true
  def handle_info({:job_event, _job_id, :console_log_event, event}, socket) do
    {
      :noreply,
      push_event(socket, "console-log-msg", %{msg: event})
    }
  end

  @impl true
  def handle_event("terminate_job", _params, socket) do
    Siplane.Job.terminate_job socket.assigns.job_id
    { :noreply, socket }
  end
end
