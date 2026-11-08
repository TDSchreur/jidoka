## Changed

- Define and Finish are no longer routed to agents on a model-routing selection when the run is `factory-fast` or the change is small (a clear, bounded request planned in the conversation, without a spec or tickets); they keep the coordinating session's model, and the stage summary says "model routing not applied" with that reason. Build and Check routing is unchanged.
